use bin_common::Transaction;
use sqlx::postgres::types::PgMoney;
use uuid::Uuid;

use crate::{
    MinilithEndpointError, MinilithResult,
    ticket::{self, AvailableAddon},
};

/// Callers hold the activity lock, also used by purchasing and ticket editing.
pub(crate) async fn save_addon(
    txn: &mut Transaction<'_>,
    activity_id: Uuid,
    addon: &AvailableAddon,
) -> MinilithResult<()> {
    let owner = sqlx::query_scalar!(
        "select activity_id from ticket_addons where id = $1",
        addon.inner.id
    )
    .fetch_optional(&mut txn.executor())
    .await?;
    if owner.is_some_and(|owner| owner != activity_id) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "addon belongs to another activity",
            "",
        ));
    }
    let locked = sqlx::query_scalar!(
        r#"select exists (
        select 1 from ticket_kind_addons links
        join ticket_kinds kind on kind.id = links.ticket_kind_id
        where links.addon_id = $1 and kind.has_been_purchased
    ) as "locked!""#,
        addon.inner.id
    )
    .fetch_one(&mut txn.executor())
    .await?;
    if locked {
        let existing = ticket::load_addons(txn, activity_id, None).await?;
        let unchanged = existing
            .iter()
            .find(|old| old.inner.id == addon.inner.id)
            .is_some_and(|old| old.immutable_fields_match(addon));
        if !unchanged {
            return Err(MinilithEndpointError::bad_frontend_code(
                "a purchased addon's structure and pricing are immutable",
                "only bookkeeping may change",
            ));
        }
        for option in &addon.options {
            let prices: Vec<PgMoney> = option
                .bookkeeping_prices
                .iter()
                .copied()
                .map(PgMoney)
                .collect();
            sqlx::query!("update ticket_addon_options set bookkeeping_prices = $2, bookkeeping_price_categories = $3 where id = $1 and ticket_addon_id = $4",
                option.id, &prices, &option.bookkeeping_price_categories, addon.inner.id)
                .execute(&mut txn.executor()).await?;
        }
        return Ok(());
    }
    sqlx::query!(r#"insert into ticket_addons (id, activity_id, idx, name, multiple_alternatives, has_text_field, required)
        values ($1, $2, coalesce((select max(idx) + 1 from ticket_addons where activity_id = $2), 0), $3, $4, $5, $6)
        on conflict (id) do update set name = excluded.name, multiple_alternatives = excluded.multiple_alternatives,
            has_text_field = excluded.has_text_field, required = excluded.required"#,
        addon.inner.id, activity_id, addon.inner.name.to_json_value(), addon.inner.multiple_alternatives,
        addon.inner.has_text_field, addon.inner.required).execute(&mut txn.executor()).await?;
    sqlx::query!(
        "delete from ticket_addon_options where ticket_addon_id = $1",
        addon.inner.id
    )
    .execute(&mut txn.executor())
    .await?;
    for (index, option) in addon.options.iter().enumerate() {
        let index = i32::try_from(index)
            .map_err(|_| MinilithEndpointError::bad_frontend_code("too many addon options", ""))?;
        let prices: Vec<PgMoney> = option
            .bookkeeping_prices
            .iter()
            .copied()
            .map(PgMoney)
            .collect();
        sqlx::query!(r#"insert into ticket_addon_options (id, ticket_addon_id, idx, name, price, bookkeeping_prices, bookkeeping_price_categories)
            values ($1, $2, $3, $4, $5, $6, $7)"#,
            option.id, addon.inner.id, index, option.name.to_json_value(), PgMoney(option.price), &prices, &option.bookkeeping_price_categories)
            .execute(&mut txn.executor()).await?;
    }
    Ok(())
}
