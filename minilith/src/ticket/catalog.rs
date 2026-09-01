use std::collections::HashMap;

use fed_auth_verifier::User;
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
            name as \"name!: DIS\", activity_id, price,
            purchasing_available_start, purchasing_available_stop,
            max_tickets, min_tickets, reserved_or_purchased_tickets,
            allow_transfer_ticket_start, allow_transfer_ticket_stop,
            has_been_purchased,
            has_been_released
        from ticket_kinds where id = $1",
        id
    )
    .map(|row| Kind {
        inner: TicketBase {
            ticket_kind_id: id,
            ticket_kind_name: row.name.0,
            activity_id: row.activity_id,
        },
        price: row.price.0,
        purchasing_available_start: row.purchasing_available_start,
        purchasing_available_stop: row.purchasing_available_stop,
        max_tickets: row.max_tickets,
        min_tickets: row.min_tickets,
        reserved_or_purchased_tickets: row.reserved_or_purchased_tickets,
        allow_transfer_ticket_start: row.allow_transfer_ticket_start,
        allow_transfer_ticket_stop: row.allow_transfer_ticket_stop,
        has_been_purchased: row.has_been_purchased,
        has_been_released: row.has_been_released,
        allowed_group_ids: Vec::new(),
        transfer_group_ids: Vec::new(),
        available_addons: Vec::new(),
    })
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;

    ticket_kind.allowed_group_ids = sqlx::query_scalar!(
        r#"select group_id from ticket_kind_allowed_groups
        where ticket_kind_id = $1 order by group_id"#,
        id,
    )
    .fetch_all(&ctx.db)
    .await?;

    ticket_kind.transfer_group_ids = sqlx::query_scalar!(
        r#"select group_id from ticket_kind_transfer_groups
        where ticket_kind_id = $1 order by group_id"#,
        id,
    )
    .fetch_all(&ctx.db)
    .await?;

    let options: HashMap<Uuid, Vec<AddonOption>> = sqlx::query!(
        "select ticket_addon_options.id, ticket_addon_id, ticket_addon_options.idx,
        ticket_addon_options.name as \"name: DIS\", price,
        -- wait wtf this Vec<i64> syntax actually works??
        bookkeeping_prices as \"bkp: Vec<i64>\", bookkeeping_price_categories
        from ticket_addon_options
        inner join ticket_addons on (ticket_addons.id = ticket_addon_options.ticket_addon_id)
        where ticket_kind_id = $1
        order by ticket_addon_options.idx",
        id
    )
    .map(|row| {
        (
            row.ticket_addon_id,
            AddonOption {
                id: row.id,
                idx: row.idx,
                name: row.name.0,
                price: row.price.0,
                bookkeeping_prices: row.bkp,
                bookkeeping_price_categories: row.bookkeeping_price_categories,
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
    ticket_kind.available_addons = sqlx::query!(
        "select id, name as \"name: DIS\",
        multiple_alternatives, has_text_field, required
        from ticket_addons
        where ticket_kind_id = $1
        order by ticket_addons.idx",
        id
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
    .fetch_all(&ctx.db)
    .await?;

    Ok(ticket_kind)
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
        "select opt.id, opt.idx, opt.name as \"name!: DIS\", opt.price,
        bookkeeping_prices as \"bp!: Vec<i64>\", bookkeeping_price_categories,
        add.id as add_id
        from purchased_tickets
        inner join ticket_kinds kind on purchased_tickets.ticket_kind_id = kind.id
        inner join ticket_addons add on add.ticket_kind_id = kind.id 
        inner join ticket_addon_options opt on opt.ticket_addon_id = add.id
        where purchased_tickets.owner_id = $1 or purchased_tickets.purchaser_id = $1",
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
                bookkeeping_price_categories: row.bookkeeping_price_categories,
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
            purchased_ticket_addons.ticket_id as "ticket_id",
            ticket_addons.id as "addon_id",
            ticket_addons.name as "addon_name: DIS",
            ticket_addons.multiple_alternatives as "multiple_alternatives",
            ticket_addons.has_text_field as "has_text_field",
            ticket_addons.required as "required",
            purchased_ticket_addons.selected_options as "selected_options",
            purchased_ticket_addons.selected_text as "selected_text"
        from purchased_tickets
        inner join purchased_ticket_addons on
            purchased_ticket_addons.ticket_id = purchased_tickets.id
        inner join ticket_addons on
            ticket_addons.id = purchased_ticket_addons.addon_id
        where purchased_tickets.owner_id = $1 or purchased_tickets.purchaser_id = $1
        order by ticket_addons.idx
        "#,
        id
    )
    .map(|row| {
        (
            row.ticket_id,
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
            purchased_tickets.id as "id",
            purchased_tickets.ticket_kind_id as "ticket_kind_id",
            ticket_kinds.activity_id as "activity_id",
            ticket_kinds.name as "ticket_kind_name!: DIS",
            activities.title as "activity_title!: DIS",
            creator.id as creator_id,
            creator.path as creator_path,
            creator.name as "creator_name!: DIS",
            activities.location as "location!: Location",
            activities.time_start as "time_start",
            activities.time_end as "time_end",
            (owner_id = $1) as "owned_by_me!"
        from purchased_tickets
        inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
        inner join activities on activities.id = ticket_kinds.activity_id
        inner join groups creator on creator.id = activities.creator_id
        where purchased_tickets.owner_id = $1 or purchased_tickets.purchaser_id = $1
        "#,
        id
    )
    .map(|ticket| PurchasedTicket {
        inner: TicketBase {
            ticket_kind_id: ticket.ticket_kind_id,
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
            from purchased_tickets
            where id = $1
            and purchaser_id = $2",
        id,
        auth.get_id(),
    )
    .fetch_optional(&ctx.db)
    .await?
    else {
        return Err(MinilithEndpointError::bad_frontend_code(
            "you cannot view the receipt if the ticket was transfered to you",
            "",
        ));
    };

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
    let receipt_lang = match lang.get(..2) {
        Some("sv") => transactions::Language::Swedish,
        _ => transactions::Language::English,
    };
    let name = ctx
        .decrypt_string(user.name)
        .wrap_err_encryption("user.name")?;

    let data = transactions::ReceiptRequest {
        language: receipt_lang,
        customer_name: name.clone(),
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
    Ok(Response::new(Binary(resp.into())).header("content-type", "application/octet-stream"))
}
