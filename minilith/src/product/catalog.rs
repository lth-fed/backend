use std::collections::HashMap;

use fed_auth_verifier::User;
use fed_auth_verifier::callbacks::TransactionState;
use minilith_errors::{MinilithErrorOptionExt as _, MinilithErrorResultExt as _};
use poem_openapi::payload::{Binary, Response};
use uuid::Uuid;

use super::models::{
    Addon, AddonOption, AvailableAddon, Kind, PurchasedAddon, PurchasedTicket, TicketBase,
};
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, MinilithEndpointError, MinilithResult,
    activities::Location, transactions,
};

/// Loads a ticket kind without checking activity access. Callers must
/// authorize the request before returning the value to a client.
#[allow(
    clippy::too_many_lines,
    reason = "keeps the existing ticket-kind query and mapping in one reusable loader"
)]
pub(crate) async fn load_ticket_kind_unchecked(
    ctx: &ContextWrapper,
    id: Uuid,
) -> MinilithResult<Kind> {
    let mut ticket_kind = sqlx::query!(
        "select
            name as \"name!: DIS\",
            (select base.price from product_base_prices base
                where base.product_id = products.id) as \"price!\",
            sales_start,
            sales_stop,
            coalesce(sales_stock, 0) as \"available!\",
            (select count(*)::int from purchases where product_id = products.id) as \"purchased_count!\",
            (select assignment.activity_id from product_activity_assignments assignment
                where assignment.product_id = products.id
                order by assignment.activity_id limit 1) as \"activity_id?\",
            resale_start, resale_stop,
            been_purchased,
            tickets_been_released
        from products where id = $1 and kind = 'ticket'",
        id
    )
    .map(|row| Kind {
        inner: TicketBase {
            ticket_kind_id: id,
            ticket_kind_name: row.name.0,
            activity_id: row.activity_id.unwrap_or_default(),
        },
        price: row.price.0,
        purchasing_available_start: row.sales_start,
        purchasing_available_stop: row.sales_stop,
        // the API keeps its old shape; a visibility ticket is a sold-out free ticket
        max_tickets: row.available + row.purchased_count,
        for_visibility: row.available == 0 && row.price.0 == 0,
        min_tickets: 0,
        reserved_or_purchased_tickets: row.purchased_count,
        allow_transfer_ticket_start: row.resale_start,
        allow_transfer_ticket_stop: row.resale_stop,
        has_been_purchased: row.been_purchased,
        has_been_released: row.tickets_been_released,
        allowed_group_ids: Vec::new(),
        transfer_group_ids: Vec::new(),
        available_addons: Vec::new(),
    })
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;

    ticket_kind.allowed_group_ids = sqlx::query_scalar!(
        r#"select group_id from product_allowed_groups
        where product_id = $1 order by group_id"#,
        id,
    )
    .fetch_all(&ctx.db)
    .await?;

    ticket_kind.transfer_group_ids = sqlx::query_scalar!(
        r#"select group_id from product_transfer_groups
        where product_id = $1 order by group_id"#,
        id,
    )
    .fetch_all(&ctx.db)
    .await?;

    ticket_kind.available_addons = load_addons(
        &mut ctx.db.begin().await?,
        ticket_kind.activity_id(),
        Some(id),
        false,
    )
    .await?;

    Ok(ticket_kind)
}

/// Loads the add-ons of one set, optionally filtered to those linked to `ticket_kind_id`.
/// `include_base` chooses whether the automatic base-price add-on is part of the result: only
/// bookkeeping needs it, customers pick from the rest.
pub(crate) async fn load_addons(
    db: &mut bin_common::Transaction<'_>,
    set_id: Uuid,
    ticket_kind_id: Option<Uuid>,
    include_base: bool,
) -> MinilithResult<Vec<AvailableAddon>> {
    let options: HashMap<Uuid, Vec<AddonOption>> = sqlx::query!(
        "select distinct product_addon_options.id, product_addon_id, product_addon_options.idx,
        product_addon_options.name as \"name: DIS\", price,
        -- wait wtf this Vec<i64> syntax actually works??
        (select coalesce(array_agg(bk.price order by bk.id), '{}'::money[])
            from product_addon_option_bookkeepings bk where bk.option_id = product_addon_options.id) as \"bkp!: Vec<i64>\",
        (select coalesce(array_agg(tag.name->>'en' order by bk.id), '{}'::text[])
            from product_addon_option_bookkeepings bk
            inner join product_tags tag on tag.id = bk.category
            where bk.option_id = product_addon_options.id) as \"bkc!: Vec<String>\"
        from product_addon_options
        inner join product_addons on (product_addons.id = product_addon_options.product_addon_id)
        left join product_addon_links links on links.addon_id = product_addons.id
        where product_addons.set_id = $1 and ($2::uuid is null or links.product_id = $2)
        order by product_addon_options.idx",
        set_id,
        ticket_kind_id
    )
    .map(|row| {
        (
            row.product_addon_id,
            AddonOption {
                id: row.id,
                idx: row.idx,
                name: row.name.0,
                price: row.price.0,
                bookkeeping_prices: row.bkp,
                bookkeeping_price_categories: row.bkc,
            },
        )
    })
    .fetch_all(&mut db.executor())
    .await?
    .into_iter()
    .fold(HashMap::new(), |mut map, (addon_id, option)| {
        map.entry(addon_id).or_default().push(option);
        map
    });
    let addons = sqlx::query!(
        "select id, name as \"name: DIS\",
        multiple_alternatives, has_text_field, required
        from product_addons
        where set_id = $1
        and ($2::uuid is null or id in (select addon_id from product_addon_links where product_id = $2))
        and ($3 or not is_base_price)
        order by product_addons.idx",
        set_id,
        ticket_kind_id,
        include_base
    )
    .map(|row| AvailableAddon {
        inner: Addon {
            id: row.id,
            name: row.name.0,
            multiple_alternatives: row.multiple_alternatives,
            has_text_field: row.has_text_field,
            required: row.required,
        },
        options: options.get(&row.id).cloned().unwrap_or_default(),
    })
    .fetch_all(&mut db.executor())
    .await?;

    Ok(addons)
}

/// The add-ons a customer may choose for a product: its activity's set (if it has one) plus its own
/// set. The base-price add-on is implicit and never offered.
pub(crate) async fn load_purchasable_addons(
    txn: &mut bin_common::Transaction<'_>,
    product_id: Uuid,
    activity_id: Option<Uuid>,
) -> MinilithResult<Vec<AvailableAddon>> {
    let mut addons = load_addons(
        txn,
        activity_id.unwrap_or(product_id),
        Some(product_id),
        false,
    )
    .await?;
    if activity_id.is_some() {
        addons.extend(load_addons(txn, product_id, Some(product_id), false).await?);
    }
    Ok(addons)
}

/// Loads a ticket kind after verifying that the user may view its activity.
pub(super) async fn get_ticket_kind(
    ctx: &ContextWrapper,
    user_id: &str,
    id: Uuid,
) -> MinilithResult<Kind> {
    let ticket_kind = load_ticket_kind_unchecked(ctx, id).await?;

    ctx.test_activity_access(user_id, &ticket_kind.activity_id())
        .await?;
    Ok(ticket_kind)
}

#[allow(clippy::too_many_lines, reason = "linear ticket and add-on mapping")]
pub(super) async fn my_tickets(
    ctx: &ContextWrapper,
    user: User,
) -> MinilithResult<Vec<PurchasedTicket>> {
    let id = user.get_id();

    let available_options: HashMap<Uuid, Vec<AddonOption>> = sqlx::query!(
        "select distinct opt.id, opt.idx, opt.name as \"name!: DIS\", opt.price,
        (select coalesce(array_agg(bk.price order by bk.id), '{}'::money[])
            from product_addon_option_bookkeepings bk where bk.option_id = opt.id) as \"bp!: Vec<i64>\",
        (select coalesce(array_agg(tag.name->>'en' order by bk.id), '{}'::text[])
            from product_addon_option_bookkeepings bk
            inner join product_tags tag on tag.id = bk.category
            where bk.option_id = opt.id) as \"bc!: Vec<String>\",
        add.id as add_id
        from purchases
        inner join products kind on purchases.product_id = kind.id
        inner join product_addon_links links on links.product_id = kind.id
        inner join product_addons add on add.id = links.addon_id
        inner join product_addon_options opt on opt.product_addon_id = add.id
        where exists (
            select 1 from purchase_transactions history
            where history.purchase_id = purchases.id
            and history.buyer_id = $1
        )",
        user.get_id()
    )
    .map(|row| {
        (
            row.add_id,
            AddonOption {
                id: row.id,
                idx: row.idx,
                name: row.name.0,
                price: row.price.0,
                bookkeeping_prices: row.bp,
                bookkeeping_price_categories: row.bc,
            },
        )
    })
    .fetch_all(&ctx.db)
    .await?
    .into_iter()
    .fold(HashMap::new(), |mut map, (addon_id, option)| {
        map.entry(addon_id).or_default().push(option);
        map
    });
    let mut addons: HashMap<Uuid, Vec<PurchasedAddon>> = sqlx::query!(
        r#"select
            purchased_product_addons.purchase_id as "purchase_id",
            product_addons.id as "addon_id",
            product_addons.name as "addon_name: DIS",
            product_addons.multiple_alternatives as "multiple_alternatives",
            product_addons.has_text_field as "has_text_field",
            product_addons.required as "required",
            purchased_product_addons.selected_options as "selected_options",
            purchased_product_addons.selected_text as "selected_text"
        from purchases
        inner join purchased_product_addons on
            purchased_product_addons.purchase_id = purchases.id
        inner join product_addons on
            product_addons.id = purchased_product_addons.addon_id
        where exists (
            select 1 from purchase_transactions history
            where history.purchase_id = purchases.id
            and history.buyer_id = $1
        )
        order by product_addons.idx
        "#,
        id
    )
    .map(|row| {
        (
            row.purchase_id,
            PurchasedAddon {
                inner: Addon {
                    id: row.addon_id,
                    name: row.addon_name.0,
                    multiple_alternatives: row.multiple_alternatives,
                    has_text_field: row.has_text_field,
                    required: row.required,
                },
                selected_options: row.selected_options,
                selected_text: row.selected_text,
                options: available_options
                    .get(&row.addon_id)
                    .cloned()
                    .unwrap_or_default(),
            },
        )
    })
    .fetch_all(&ctx.db)
    .await?
    .into_iter()
    .fold(HashMap::new(), |mut map, (ticket_id, addon)| {
        map.entry(ticket_id).or_default().push(addon);
        map
    });

    let tickets = sqlx::query!(
        r#"select
            purchases.id as "id",
            purchases.product_id as "product_id",
            assignment.activity_id as "activity_id",
            products.name as "ticket_kind_name!: DIS",
            activities.title as "activity_title!: DIS",
            creator.id as creator_id,
            creator.path as creator_path,
            creator.name as "creator_name!: DIS",
            activities.location as "location!: Location",
            activities.time_start as "time_start",
            activities.time_end as "time_end",
            (owner_id = $1) as "owned_by_me!",
            case when purchases.owner_id = $1 then
                (select used.id from purchase_uses used
                    where used.purchase_id = purchases.id
                    order by used.timestamp desc, used.id desc
                    limit 1)
            end as "last_use_id?"
        from purchases
        inner join products on products.id = purchases.product_id
        inner join product_activity_assignments assignment on assignment.product_id = products.id
        inner join activities on activities.id = assignment.activity_id
        inner join groups creator on creator.id = products.group_id
        where exists (
            select 1 from purchase_transactions history
            where history.purchase_id = purchases.id
            and history.buyer_id = $1
        )
        "#,
        id
    )
    .map(|ticket| PurchasedTicket {
        inner: TicketBase {
            ticket_kind_id: ticket.product_id,
            ticket_kind_name: ticket.ticket_kind_name.0,
            activity_id: ticket.activity_id,
        },
        id: ticket.id,
        activity_location: ticket.location.into(),
        activity_title: ticket.activity_title.0,
        creator_id: ticket.creator_id,
        creator_path: ticket.creator_path.to_string(),
        creator_name: ticket.creator_name.0,
        time_start: ticket.time_start,
        time_end: ticket.time_end,
        purchased_addons: addons.remove(&ticket.id).unwrap_or_default(),
        last_use_id: ticket.last_use_id,
        owned_by_me: ticket.owned_by_me,
    })
    .fetch_all(&ctx.db)
    .await?;

    Ok(tickets)
}
pub(super) async fn receipt(
    ctx: &ContextWrapper,
    auth: User,
    id: Uuid,
) -> MinilithResult<Response<Binary<poem::Body>>> {
    let Some(transaction_id) = sqlx::query_scalar!(
        "select transaction_id
            from purchase_transactions
            where purchase_id = $1
            and buyer_id = $2
            and transaction_id is not null
            order by completed_at
            limit 1",
        id,
        auth.get_id(),
    )
    .fetch_optional(&ctx.db)
    .await?
    .flatten() else {
        return Err(MinilithEndpointError::bad_frontend_code(
            "you cannot view the receipt if the ticket was transfered to you",
            "",
        ));
    };

    // A refund callback may have been lost, so reconcile with the source of truth before
    // serving a receipt. Only the current owner's own payment can mark the purchase refunded; a
    // resale refunds the previous owner's older transaction.
    if let Ok(resp) = ctx
        .transactions_post("/v0/info")
        .json(&transactions::InfoRequest {
            transaction_ids: vec![transaction_id],
        })
        .send()
        .await
        && let Ok(infos) = resp.json::<Vec<transactions::SingleInfoResponse>>().await
        && infos
            .iter()
            .any(|info| matches!(info.state, TransactionState::Refunded))
    {
        sqlx::query!(
            "update purchases set refunded_id = $2
            where id = $1
            and owner_id = $3
            and refunded_id is null",
            id,
            transaction_id,
            auth.get_id(),
        )
        .execute(&ctx.db)
        .await?;
    }

    let user = sqlx::query!(
        "select name, language
            from users where id = $1",
        auth.get_id()
    )
    .fetch_one(&ctx.db)
    .await?;

    let lang = ctx
        .decrypt_string(user.language)
        .wrap_err_encryption("user.language")?;
    let name = ctx
        .decrypt_string(user.name)
        .wrap_err_encryption("user.name")?;

    let pdf = receipt_pdf(ctx, transaction_id, receipt_language(&lang), &name).await?;
    Ok(Response::new(Binary(pdf.into())).header("content-type", "application/octet-stream"))
}

/// The receipt's language: a user's language tag, Swedish for `sv` and English otherwise.
pub(crate) fn receipt_language(language: &str) -> transactions::Language {
    match language.get(..2) {
        Some("sv") => transactions::Language::Swedish,
        _ => transactions::Language::English,
    }
}

/// The transaction the current owner paid with, i.e. the one whose receipt they can get. `None`
/// when the owner never paid (the purchase was transferred to them).
pub(crate) async fn owner_transaction(
    ctx: &ContextWrapper,
    purchase_id: Uuid,
) -> MinilithResult<Option<Uuid>> {
    Ok(sqlx::query_scalar!(
        r#"select transaction_id
        from purchase_transactions
        where purchase_id = $1
        and transaction_id is not null
        and buyer_id = (select owner_id from purchases where id = $1)
        order by completed_at desc
        limit 1"#,
        purchase_id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .flatten())
}

/// Renders the provider's receipt PDF for one transaction.
pub(crate) async fn receipt_pdf(
    ctx: &ContextWrapper,
    transaction_id: Uuid,
    language: transactions::Language,
    customer_name: &str,
) -> MinilithResult<Vec<u8>> {
    let data = transactions::ReceiptRequest {
        language,
        customer_name: customer_name.to_owned(),
    };
    let resp = ctx
        .transactions_post(format!("/v0/{transaction_id}/receipt"))
        .json(&data)
        .send()
        .await
        .wrap_err_internal("receipt failed to fetch")?
        .error_for_status()
        .wrap_err_internal("receipt status code error")?
        .bytes()
        .await
        .wrap_err_internal("receipt read body")?;
    Ok(resp.to_vec())
}
