use std::collections::HashMap;

use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use poem_openapi::Object;
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use super::apple_pass::verify_barcode_signature;
use super::models::{Addon, AddonOption, PurchasedAddon};
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, InternationalizedString as IS, MinilithResult,
};

#[derive(Object)]
pub struct ValidateActivity {
    id: Uuid,
    title: IS,
    description: IS,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
}

/// What a scanned QR code says. Exactly one of `created_at` and `pass_signature` is needed:
///
/// - The app's own code, `{"tid", "t"}`, changes every second, so a screenshot goes stale. Send
///   `t` as `created_at`.
/// - A wallet pass's code, `{"tid", "sig"}` (`/v1/purchases/{id}/apple-pass`), can't change, so it is
///   signed for the ticket's current owner instead and stops validating once the ticket is
///   transferred. Send `sig` as `pass_signature`.
#[derive(Object)]
pub struct ValidateRequest {
    purchased_ticket_id: Uuid,
    created_at: Option<OffsetDateTime>,
    pass_signature: Option<String>,
}
#[derive(Object)]
pub struct Validation {
    at: OffsetDateTime,
}
#[derive(Object)]
pub struct ValidateResponse {
    verified: bool,
    ticket_kind_name: IS,
    owner_id: Option<String>,
    owner_name: Option<String>,
    has_been_transfered: bool,
    purchaser_name: Option<String>,
    previous_verifications: Vec<Validation>,
    purchased_addons: Vec<PurchasedAddon>,
}
impl ValidateResponse {
    pub fn not_valid() -> Self {
        Self {
            verified: false,
            ticket_kind_name: IS::empty(),
            owner_id: None,
            owner_name: None,
            has_been_transfered: false,
            purchaser_name: None,
            previous_verifications: vec![],
            purchased_addons: vec![],
        }
    }
}

pub(super) async fn validatable_activities(
    ctx: &ContextWrapper,
    auth: User,
) -> MinilithResult<Vec<ValidateActivity>> {
    sqlx::query!(
        "select title as \"title!: DIS\", description as \"description!: DIS\",
            a.id, url, time_start, time_end
            from activity_verifiers
            inner join activities a on a.id = activity_verifiers.activity_id
            inner join images on images.id = a.image_id
            where user_id = $1
            and a.time_end > now() - '24 hours'::interval",
        auth.get_id()
    )
    .map(|row| ValidateActivity {
        id: row.id,
        title: row.title.0,
        description: row.description.0,
        time_start: row.time_start,
        time_end: row.time_end,
        image_url: row.url,
    })
    .fetch_all(&ctx.db)
    .await
    .map_err(Into::into)
}
#[allow(
    clippy::too_many_lines,
    reason = "linear validation and response mapping"
)]
pub(super) async fn validate(
    ctx: &ContextWrapper,
    auth: User,
    body: ValidateRequest,
) -> MinilithResult<ValidateResponse> {
    let now = OffsetDateTime::now_utc();
    // TODO(frontend-hack: 25/08/2026): some people are insane and don't have accurate time on their phone, so we have to
    // increase leeway before we implement server side time adjustment
    let leeway = 5 * time::Duration::MINUTE;
    match (body.created_at, &body.pass_signature) {
        // Checked below, once the owner is known.
        (_, Some(_)) => {}
        (Some(created_at), None)
            if created_at >= now.saturating_sub(leeway)
                && created_at <= now.saturating_add(leeway) => {}
        _ => return Ok(ValidateResponse::not_valid()),
    }
    let Some(row) = sqlx::query!(
        "select owner_id, purchaser_id, owner.name as oname, purchaser.name as pname,
            purchases.product_id as \"product_id!\", kind.name as \"ticket_kind_name: DIS\",
            (purchases.refunded_id is not null) as \"refunded!\"
            from purchases
            inner join products kind on kind.id = purchases.product_id
            inner join product_activity_assignments assignment
                on assignment.product_id = purchases.product_id
            inner join activity_verifiers
                on activity_verifiers.activity_id = assignment.activity_id
            inner join users owner on owner.id = owner_id
            inner join users purchaser on purchaser.id = purchaser_id
            where purchases.id = $1
                and activity_verifiers.user_id = $2",
        body.purchased_ticket_id,
        auth.get_id()
    )
    .fetch_optional(&ctx.db)
    .await?
    else {
        return Ok(ValidateResponse::not_valid());
    };
    if let Some(signature) = &body.pass_signature
        && !verify_barcode_signature(ctx, body.purchased_ticket_id, &row.owner_id, signature)?
    {
        // A pass made for an earlier owner, or a forged one.
        return Ok(ValidateResponse::not_valid());
    }
    // A refunded purchase must not be usable.
    if row.refunded {
        return Ok(ValidateResponse::not_valid());
    }
    let previous_verifications = sqlx::query!(
        "select validation.timestamp
        from purchase_use_validations validation
        inner join purchase_uses uses on uses.id = validation.use_id
        where uses.purchase_id = $1
        order by validation.timestamp",
        body.purchased_ticket_id
    )
    .map(|row| Validation { at: row.timestamp })
    .fetch_all(&ctx.db)
    .await?;
    let mut available_options: HashMap<Uuid, Vec<AddonOption>> = sqlx::query!(
        "select opt.id, opt.idx, opt.name as \"name!: DIS\", opt.price,
            (select coalesce(array_agg(bk.price order by bk.id), '{}'::money[])
                from product_addon_option_bookkeepings bk where bk.option_id = opt.id) as \"bp!: Vec<i64>\",
            (select coalesce(array_agg(tag.name->>'en' order by bk.id), '{}'::text[])
                from product_addon_option_bookkeepings bk
                inner join product_tags tag on tag.id = bk.category
                where bk.option_id = opt.id) as \"bc!: Vec<String>\",
            add.id as add_id
            from products kind
            inner join product_addon_links links on links.product_id = kind.id
            inner join product_addons add on add.id = links.addon_id
            inner join product_addon_options opt on opt.product_addon_id = add.id
            where kind.id = $1",
        row.product_id
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
    let purchased_addons: Vec<PurchasedAddon> = sqlx::query!(
        r#"select
                purchased_product_addons.purchase_id as "purchase_id",
                product_addons.id as "addon_id",
                product_addons.name as "addon_name: DIS",
                product_addons.multiple_alternatives as "multiple_alternatives",
                product_addons.has_text_field as "has_text_field",
                product_addons.required as "required",
                purchased_product_addons.selected_options as "selected_options",
                purchased_product_addons.selected_text as "selected_text"
            from purchased_product_addons
            inner join product_addons on
                product_addons.id = purchased_product_addons.addon_id
            where purchased_product_addons.purchase_id = $1
            order by product_addons.idx
            "#,
        body.purchased_ticket_id
    )
    .map(|row| PurchasedAddon {
        inner: Addon {
            id: row.addon_id,
            name: row.addon_name.0,
            multiple_alternatives: row.multiple_alternatives,
            has_text_field: row.has_text_field,
            required: row.required,
        },
        selected_options: row.selected_options,
        selected_text: row.selected_text,
        options: available_options.remove(&row.addon_id).unwrap_or_default(),
    })
    .fetch_all(&ctx.db)
    .await?;
    let use_id = sqlx::query_scalar!(
        "insert into purchase_uses (id, purchase_id) values ($1, $2) returning id",
        Uuid::new_v4(),
        body.purchased_ticket_id
    )
    .fetch_one(&ctx.db)
    .await?;
    sqlx::query!(
        "insert into purchase_use_validations (id, use_id) values ($1, $2)",
        Uuid::new_v4(),
        use_id
    )
    .execute(&ctx.db)
    .await?;
    Ok(ValidateResponse {
        verified: true,
        ticket_kind_name: row.ticket_kind_name.0,
        has_been_transfered: row.owner_id != row.purchaser_id,
        owner_id: Some(row.owner_id),
        owner_name: Some(
            ctx.decrypt_string(row.oname)
                .wrap_err_encryption("validate name")?,
        ),
        purchaser_name: Some(
            ctx.decrypt_string(row.pname)
                .wrap_err_encryption("validate purchaser name")?,
        ),
        previous_verifications,
        purchased_addons,
    })
}
