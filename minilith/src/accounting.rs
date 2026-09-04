use std::collections::{BTreeMap, HashMap};

use minilith_errors::{MinilithErrorResultExt as _, MinilithResult};
use tracing::error;
use uuid::Uuid;

use crate::{
    ContextWrapper, DbInternationalizedString as DIS, MinilithErrorOptionExt as _, report, ticket,
    transactions,
};

#[derive(Debug)]
struct PurchasedTicket {
    id: Uuid,
    ticket_kind_id: Uuid,
    transaction_id: Uuid,
}

#[derive(Debug)]
struct PurchasedAddon {
    addon_id: Uuid,
    selected_options: Vec<i32>,
}

pub(crate) struct GeneratedReport {
    pub activity_name: String,
    pub pdf: Vec<u8>,
}

async fn fetch_receipts(
    ctx: &ContextWrapper,
    activity_id: Uuid,
) -> MinilithResult<Vec<bytes::Bytes>> {
    let transactions = sqlx::query!(
        r#"select distinct on (purchased_tickets.transaction_id)
            purchased_tickets.transaction_id,
            users.name,
            users.language
        from purchased_tickets
        inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
        inner join users on users.id = purchased_tickets.purchaser_id
        where ticket_kinds.activity_id = $1
        order by purchased_tickets.transaction_id, purchased_tickets.id"#,
        activity_id,
    )
    .fetch_all(&ctx.db)
    .await?;

    let mut receipts = Vec::with_capacity(transactions.len());
    for transaction in transactions {
        let language = ctx
            .decrypt_string(transaction.language)
            .wrap_err_encryption("accounting receipt user language")?;
        let language = match language.split('-').next() {
            Some("sv") => transactions::Language::Swedish,
            _ => transactions::Language::English,
        };
        let customer_name = ctx
            .decrypt_string(transaction.name)
            .wrap_err_encryption("accounting receipt customer name")?;
        let receipt = ctx
            .transactions_post(format!("/v0/{}/receipt", transaction.transaction_id))
            .json(&transactions::ReceiptRequest {
                language,
                customer_name,
            })
            .send()
            .await
            .wrap_err_internal("accounting: failed to fetch receipt")?
            .error_for_status()
            .wrap_err_internal("accounting: receipt returned non-2xx")?
            .bytes()
            .await
            .wrap_err_internal("accounting: failed to read receipt")?;
        receipts.push(receipt);
    }
    Ok(receipts)
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
    append_receipts: bool,
) -> MinilithResult<GeneratedReport> {
    let lang = match language {
        report::Language::Swedish => "sv",
        report::Language::English => "en",
    };
    let activity = sqlx::query!(
        r#"select
            activities.title as "title!: DIS",
            creator.name as "creator_name!: DIS",
            images.url as creator_logo_url
        from activities
        inner join groups creator on creator.id = activities.creator_id
        inner join images on images.id = creator.logo_id
        where activities.id = $1"#,
        activity_id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;

    let extension = activity
        .creator_logo_url
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
        let path = activity
            .creator_logo_url
            .rsplit('/')
            .next()
            .unwrap_or(&activity.creator_logo_url);
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

    let ticket_kind_ids = sqlx::query_scalar!(
        "select id from ticket_kinds where activity_id = $1",
        activity_id,
    )
    .fetch_all(&ctx.db)
    .await?;
    let mut kinds = HashMap::with_capacity(ticket_kind_ids.len());
    for ticket_kind_id in ticket_kind_ids {
        kinds.insert(
            ticket_kind_id,
            ticket::load_ticket_kind_unchecked(ctx, ticket_kind_id).await?,
        );
    }

    let tickets = sqlx::query_as!(
        PurchasedTicket,
        r#"select purchased_tickets.id, purchased_tickets.ticket_kind_id,
            purchased_tickets.transaction_id
        from purchased_tickets
        inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
        where ticket_kinds.activity_id = $1
        order by purchased_tickets.id"#,
        activity_id,
    )
    .fetch_all(&ctx.db)
    .await?;
    let addons = sqlx::query!(
        r#"select purchased_ticket_addons.ticket_id,
            purchased_ticket_addons.addon_id,
            purchased_ticket_addons.selected_options
        from purchased_ticket_addons
        inner join purchased_tickets
            on purchased_tickets.id = purchased_ticket_addons.ticket_id
        inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
        where ticket_kinds.activity_id = $1"#,
        activity_id,
    )
    .map(|row| {
        (
            row.ticket_id,
            PurchasedAddon {
                addon_id: row.addon_id,
                selected_options: row.selected_options,
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

    let transaction_ids = tickets
        .iter()
        .map(|ticket| ticket.transaction_id)
        .collect::<Vec<_>>();
    let fees = if transaction_ids.is_empty() {
        0
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
            .map(|info| info.total_fees)
            .sum()
    };

    let mut per_object = BTreeMap::new();
    let mut per_alcohol_category = BTreeMap::new();
    for purchased_ticket in &tickets {
        let kind = kinds
            .get(&purchased_ticket.ticket_kind_id)
            .wrap_err_internal("accounting: purchased ticket kind is missing")?;
        let kind_name = kind.inner.ticket_kind_name.resolve_intl(lang, "");
        per_object
            .entry((report::Kind::Ticket, kind_name.to_owned()))
            .or_insert((kind.price, 0))
            .1 += 1;
        *per_alcohol_category.entry("null".to_owned()).or_insert(0) += kind.price;

        for addon in addons.get(&purchased_ticket.id).into_iter().flatten() {
            let addon_data = kind
                .available_addons
                .iter()
                .find(|available| available.inner.id == addon.addon_id)
                .wrap_err_internal("accounting: purchased ticket addon is missing")?;
            for selected_option in &addon.selected_options {
                let option = addon_data
                    .options
                    .iter()
                    .find(|option| option.idx == *selected_option)
                    .wrap_err_internal("accounting: purchased ticket addon option is missing")?;
                let name = format!(
                    "{} - {}",
                    addon_data.inner.name.resolve_intl(lang, ""),
                    option.name.resolve_intl(lang, "")
                );
                per_object
                    .entry((report::Kind::Option, name))
                    .or_insert((option.price, 0))
                    .1 += 1;
                for (category, price) in option
                    .bookkeeping_price_categories
                    .iter()
                    .zip(option.bookkeeping_prices.iter())
                {
                    *per_alcohol_category.entry(category.clone()).or_insert(0) += *price;
                }
            }
        }
    }
    for (category, total) in external_sales {
        per_object
            .entry((report::Kind::External, String::new()))
            .or_insert((0, 1))
            .0 += total;
        *per_alcohol_category.entry(category).or_insert(0) += total;
    }

    let receipts = if append_receipts {
        fetch_receipts(ctx, activity_id).await?
    } else {
        Vec::new()
    };
    let activity_name = activity.title.resolve_intl(lang, "<title>").to_owned();
    let data = report::Data {
        language,
        activity_name: activity_name.clone(),
        creator_name: activity.creator_name.resolve_intl(lang, "").to_owned(),
        creator_logo_format: image_format,
        creator_logo_data: image_data,
        fees,
        fees_external: external_sale_fees,
        per_object: per_object
            .into_iter()
            .map(|((kind, name), (price, number))| report::Object {
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
        receipt_count: receipts.len(),
        receipts,
    };
    let pdf = report::compile(ctx.report_typst(), &data)?;
    Ok(GeneratedReport { activity_name, pdf })
}
