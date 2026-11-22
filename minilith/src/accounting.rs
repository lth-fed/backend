use std::collections::{BTreeMap, HashMap};

use minilith_errors::{MinilithErrorResultExt as _, MinilithResult};
use sqlx::postgres::types::PgMoney;
use tracing::error;
use uuid::Uuid;

use crate::{
    ContextWrapper, DbInternationalizedString as DIS, MinilithErrorOptionExt as _, product, report,
    transactions,
};

#[derive(Debug)]
struct PurchasedTicket {
    id: Uuid,
    ticket_kind_id: Uuid,
    transaction_id: Uuid,
    /// Set by an admin refund of the purchase; resales never set it.
    refunded_id: Option<Uuid>,
    product_price: PgMoney,
    /// What the current owner paid (ticket + addons; the resale price after a resale).
    paid_price: PgMoney,
}

impl PurchasedTicket {
    fn is_refunded(&self) -> bool {
        self.refunded_id.is_some()
    }
}

struct ReportProduct {
    name: crate::InternationalizedString,
    addons: Vec<product::AvailableAddon>,
}

#[derive(Debug)]
struct PurchasedAddon {
    addon_id: Uuid,
    selected_options: Vec<i32>,
    selected_options_prices: Vec<PgMoney>,
}

pub(crate) struct GeneratedReport {
    pub activity_name: String,
    pub total_sales: i64,
    pub pdf: Vec<u8>,
}

#[allow(
    clippy::too_many_lines,
    reason = "the report calculation is linear and mirrors the bookkeeping document"
)]
pub(crate) async fn generate_activity_report(
    ctx: &ContextWrapper,
    activity_id: Uuid,
    language: report::Language,
    external_sales: Vec<(String, i64)>,
    external_sale_fees: i64,
    bookkeeping_export: bool,
) -> MinilithResult<GeneratedReport> {
    let activity = sqlx::query!(
        r#"select
            activities.title as "title!: DIS",
            creator.name as "creator_name!: DIS",
            images.url as creator_logo_url
        from activities
        inner join sales_targets target on target.id = activities.sales_target_id
        inner join groups creator on creator.id = target.group_id
        inner join images on images.id = creator.logo_id
        where activities.id = $1"#,
        activity_id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;

    let activity_name = activity
        .title
        .resolve_intl(language_code(language), "<title>")
        .to_owned();
    generate_report(
        ctx,
        Some(activity_id),
        None,
        None,
        None,
        language,
        activity_name,
        activity.creator_name.0,
        activity.creator_logo_url,
        external_sales,
        external_sale_fees,
        bookkeeping_export,
    )
    .await
}

/// Group reports include only purchases of products owned by the group, in the half-open period.
#[allow(
    clippy::too_many_arguments,
    reason = "report parameters describe the export scope"
)]
pub(crate) async fn generate_group_report(
    ctx: &ContextWrapper,
    group_id: Uuid,
    start: Option<time::OffsetDateTime>,
    end: Option<time::OffsetDateTime>,
    activity_id: Option<Uuid>,
    language: report::Language,
) -> MinilithResult<GeneratedReport> {
    let group = sqlx::query!(
        r#"select groups.name as "name!: DIS", images.url as logo_url
        from groups inner join images on images.id = groups.logo_id where groups.id = $1"#,
        group_id
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;
    let fmt = |date: Option<time::OffsetDateTime>| {
        date.map_or_else(|| "…".to_owned(), |value| value.to_string())
    };
    let period = format!("{} – {}", fmt(start), fmt(end));
    generate_report(
        ctx,
        activity_id,
        Some(group_id),
        start,
        end,
        language,
        period,
        group.name.0,
        group.logo_url,
        Vec::new(),
        0,
        false,
    )
    .await
}

fn language_code(language: report::Language) -> &'static str {
    match language {
        report::Language::Swedish => "sv",
        report::Language::English => "en",
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "shared report calculation"
)]
async fn generate_report(
    ctx: &ContextWrapper,
    activity_id: Option<Uuid>,
    group_id: Option<Uuid>,
    start: Option<time::OffsetDateTime>,
    end: Option<time::OffsetDateTime>,
    language: report::Language,
    activity_name: String,
    creator_name: crate::InternationalizedString,
    creator_logo_url: String,
    external_sales: Vec<(String, i64)>,
    external_sale_fees: i64,
    bookkeeping_export: bool,
) -> MinilithResult<GeneratedReport> {
    let lang = language_code(language);
    let extension = creator_logo_url
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase);
    let mut image_format = match extension.as_deref() {
        Some("jpg" | "jpeg") => Some(report::ImageKind::Jpg),
        Some("png") => Some(report::ImageKind::Png),
        Some("svg") => Some(report::ImageKind::Svg),
        Some("webp") => Some(report::ImageKind::Webp),
        _ => None,
    };
    let image_data = if image_format.is_some() {
        let path = creator_logo_url
            .rsplit('/')
            .next()
            .unwrap_or(&creator_logo_url);
        match ctx.image_bucket().get_object(path).await {
            Err(err) => {
                error!(error = ?err, "s3: failed to get creator icon for accounting report");
                image_format = None;
                None
            }
            Ok(response) => Some(response.into_bytes()),
        }
    } else {
        None
    };

    let tickets = sqlx::query_as!(
        PurchasedTicket,
        r#"select purchases.id, purchases.product_id as ticket_kind_id,
            purchases.product_price as "product_price!",
            purchases.paid_price as "paid_price!",
            (select transaction_id from purchase_transactions
                where purchase_id = purchases.id and transaction_id is not null
                order by completed_at limit 1) as "transaction_id!",
            purchases.refunded_id
        from purchases
        inner join products on products.id = purchases.product_id
        where ($1::uuid is null or purchases.activity_id = $1)
        and (case when $2::uuid is null then products.kind = 'ticket' else products.group_id = $2 end)
        and ($3::timestamptz is null or purchases.purchased_at >= $3)
        and ($4::timestamptz is null or purchases.purchased_at < $4)
        order by purchases.id"#,
        activity_id, group_id, start, end,
    )
    .fetch_all(&ctx.db)
    .await?;
    let product_ids = tickets
        .iter()
        .map(|ticket| ticket.ticket_kind_id)
        .collect::<std::collections::HashSet<_>>();
    let mut kinds = HashMap::with_capacity(product_ids.len());
    for id in product_ids {
        let row = sqlx::query!(
            r#"select name as "name!: DIS" from products where id = $1"#,
            id
        )
        .fetch_one(&ctx.db)
        .await?;
        // A product can have add-ons from multiple sets; load each linked set.
        let sets = sqlx::query_scalar!(
            "select distinct addon.set_id from product_addons addon inner join product_addon_links link on link.addon_id = addon.id where link.product_id = $1", id
        ).fetch_all(&ctx.db).await?;
        let mut available = Vec::new();
        for set in sets {
            available
                .extend(product::load_addons(&mut ctx.db.begin().await?, set, Some(id)).await?);
        }
        kinds.insert(
            id,
            ReportProduct {
                name: row.name.0,
                addons: available,
            },
        );
    }
    let addons = sqlx::query!(
        r#"select purchased_product_addons.purchase_id,
            purchased_product_addons.addon_id,
            purchased_product_addons.selected_options,
            purchased_product_addons.selected_options_prices
        from purchased_product_addons
        inner join purchases
            on purchases.id = purchased_product_addons.purchase_id
        inner join products on products.id = purchases.product_id
        where ($1::uuid is null or purchases.activity_id = $1)
        and (case when $2::uuid is null then products.kind = 'ticket' else products.group_id = $2 end)
        and ($3::timestamptz is null or purchases.purchased_at >= $3)
        and ($4::timestamptz is null or purchases.purchased_at < $4)"#,
        activity_id, group_id, start, end,
    )
    .map(|row| {
        (
            row.purchase_id,
            PurchasedAddon {
                addon_id: row.addon_id,
                selected_options: row.selected_options,
                selected_options_prices: row.selected_options_prices,
            },
        )
    })
    .fetch_all(&ctx.db)
    .await?
    .into_iter()
    .fold(
        HashMap::<Uuid, Vec<PurchasedAddon>>::new(),
        |mut addons, (ticket_id, addon)| {
            addons.entry(ticket_id).or_default().push(addon);
            addons
        },
    );

    // Fees are only ever taken from the purchase's first transaction. For purchases that were
    // refunded we additionally need the refunded transaction's provider to quote the refund fee.
    let mut transaction_ids = tickets
        .iter()
        .map(|ticket| ticket.transaction_id)
        .collect::<Vec<_>>();
    transaction_ids.extend(
        tickets
            .iter()
            .filter(|ticket| ticket.is_refunded())
            .filter_map(|ticket| ticket.refunded_id),
    );
    transaction_ids.sort_unstable();
    transaction_ids.dedup();
    let transaction_info = if transaction_ids.is_empty() {
        HashMap::new()
    } else {
        ctx.transactions_post("/v0/info")
            .json(&transactions::InfoRequest { transaction_ids })
            .send()
            .await
            .wrap_err_internal("accounting: failed to get transaction info")?
            .error_for_status()
            .wrap_err_internal("accounting: transaction info returned non-2xx")?
            .json::<Vec<transactions::SingleInfoResponse>>()
            .await
            .wrap_err_internal("accounting: failed to parse transaction info")?
            .into_iter()
            .map(|info| (info.id, info))
            .collect::<HashMap<_, _>>()
    };
    let fees: i64 = tickets
        .iter()
        .filter_map(|ticket| transaction_info.get(&ticket.transaction_id))
        .map(|info| info.total_fees)
        .sum();
    let mut fees_refund: i64 = 0;
    let mut swish_refund_fee: Option<i64> = None;

    let mut per_object = BTreeMap::new();
    let mut per_refund = BTreeMap::new();
    let mut per_alcohol_category = BTreeMap::new();
    for purchased_ticket in &tickets {
        let kind = kinds
            .get(&purchased_ticket.ticket_kind_id)
            .wrap_err_internal("accounting: purchased ticket kind is missing")?;
        let kind_name = kind.name.resolve_intl(lang, "");
        // The price actually paid is snapshotted on the purchase; the product's price may have
        // changed since (or been discounted), so only the snapshot counts as income.
        let ticket_price = purchased_ticket.product_price.0;
        *per_object
            .entry((report::Kind::Ticket, kind_name.to_owned(), ticket_price))
            .or_insert(0) += 1;
        let mut contributions = vec![("null".to_owned(), ticket_price)];

        for addon in addons.get(&purchased_ticket.id).into_iter().flatten() {
            let addon_data = kind
                .addons
                .iter()
                .find(|available| available.inner.id == addon.addon_id)
                .wrap_err_internal("accounting: purchased ticket addon is missing")?;
            for (selected_option, selected_price) in addon
                .selected_options
                .iter()
                .zip(addon.selected_options_prices.iter())
            {
                let option = addon_data
                    .options
                    .iter()
                    .find(|option| option.idx == *selected_option)
                    .wrap_err_internal("accounting: purchased ticket addon option is missing")?;
                let price = selected_price.0;
                let name = format!(
                    "{} - {}",
                    addon_data.inner.name.resolve_intl(lang, ""),
                    option.name.resolve_intl(lang, "")
                );
                *per_object
                    .entry((report::Kind::Option, name, price))
                    .or_insert(0) += 1;
                contributions.extend(allocate_across_categories(
                    price,
                    &option.bookkeeping_price_categories,
                    &option.bookkeeping_prices,
                ));
            }
        }

        if purchased_ticket.is_refunded() {
            let refunded = purchased_ticket
                .refunded_id
                .and_then(|id| transaction_info.get(&id));
            // A resold purchase is refunded at what its owner paid, not at the original price.
            let refunded_amount = purchased_ticket.paid_price.0;
            let provider = refunded.and_then(|info| info.provider);
            fees_refund +=
                refund_fee(ctx, provider, refunded_amount, &mut swish_refund_fee).await?;
            // A resale's transaction also contains fee wares (the refund fee for the previous
            // owner and its own payment fee). A full refund returns those to the buyer, but the
            // provider kept its cut and the earlier refund fee is spent, so they are a cost of
            // this refund rather than product income.
            fees_refund += refunded.map_or(0, |info| (info.total - refunded_amount).max(0));
            *per_refund
                .entry((report::Kind::Refund, kind_name.to_owned(), refunded_amount))
                .or_insert(0) += 1;
            for (category, amount) in contributions {
                *per_alcohol_category.entry(category).or_insert(0) -= amount;
            }
        } else {
            for (category, amount) in contributions {
                *per_alcohol_category.entry(category).or_insert(0) += amount;
            }
        }
    }
    let total_sales = per_object
        .iter()
        .fold(0, |acc, ((_, _, price), number)| acc + price * number);
    for (category, total) in external_sales {
        *per_object
            .entry((report::Kind::External, category.clone(), total))
            .or_insert(0) += 1;
        *per_alcohol_category.entry(category).or_insert(0) += total;
    }

    let data = report::Data {
        language,
        activity_name: activity_name.clone(),
        is_group: group_id.is_some(),
        creator_name: creator_name.resolve_intl(lang, "").to_owned(),
        creator_logo_format: image_format,
        creator_logo_data: image_data,
        fees,
        fees_external: external_sale_fees,
        fees_refund,
        per_object: per_object
            .into_iter()
            .map(|((kind, name, price), number)| report::Object {
                name,
                kind,
                price,
                number,
            })
            .collect(),
        per_refund: per_refund
            .into_iter()
            .map(|((kind, name, price), number)| report::Object {
                name,
                kind,
                price,
                number,
            })
            .collect(),
        per_alcohol_category: per_alcohol_category
            .into_iter()
            .map(|(name, amount)| report::AlcoholCategory { name, amount })
            .collect(),
        exported_at: bookkeeping_export.then(|| {
            time::OffsetDateTime::now_utc()
                .date()
                .format(&time::format_description::well_known::Iso8601::DATE)
                .unwrap_or_default()
        }),
    };
    let pdf = report::compile(ctx.report_typst(), &data)?;
    Ok(GeneratedReport {
        activity_name,
        total_sales,
        pdf,
    })
}

/// Splits the paid price across an addon option's bookkeeping categories. The declared prices may
/// be stale (the purchase snapshots the price it was actually sold for), so they are used as
/// weights and the rounding remainder is added to the first category.
fn allocate_across_categories(
    price: i64,
    categories: &[String],
    weights: &[i64],
) -> Vec<(String, i64)> {
    let total_weight: i64 = weights.iter().sum();
    if total_weight <= 0 {
        return Vec::new();
    }
    let mut allocated = 0;
    let mut shares = categories
        .iter()
        .zip(weights)
        .map(|(category, weight)| {
            let share = price * weight / total_weight;
            allocated += share;
            (category.clone(), share)
        })
        .collect::<Vec<_>>();
    if let Some((_, first)) = shares.first_mut() {
        *first += price - allocated;
    }
    shares
}

/// Quotes the provider's refund fee. `Swish` charges a flat fee per refund and everything else is
/// free, so a single quote per report is cached.
async fn refund_fee(
    ctx: &ContextWrapper,
    provider: Option<transactions::Provider>,
    amount: i64,
    cached: &mut Option<i64>,
) -> MinilithResult<i64> {
    match provider {
        Some(transactions::Provider::Swish) => {
            if let Some(fee) = *cached {
                return Ok(fee);
            }
            let fee = product::fee_quote(
                ctx,
                product::PurchaseProvider::Swish,
                amount,
                transactions::FeeKind::Refund,
            )
            .await?;
            *cached = Some(fee);
            Ok(fee)
        }
        Some(
            transactions::Provider::Sumup
            | transactions::Provider::Free
            | transactions::Provider::Stripe,
        )
        | None => Ok(0),
    }
}
