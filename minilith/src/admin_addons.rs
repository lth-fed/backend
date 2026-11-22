use bin_common::Transaction;
use sqlx::postgres::types::PgMoney;
use uuid::Uuid;

use crate::{
    MinilithEndpointError, MinilithResult,
    product::{self, AvailableAddon},
};

/// Callers hold the activity lock, also used by purchasing and ticket editing.
pub(crate) async fn save_addon(
    txn: &mut Transaction<'_>,
    set_id: Uuid,
    addon: &AvailableAddon,
) -> MinilithResult<()> {
    let owner = sqlx::query_scalar!(
        "select set_id from product_addons where id = $1",
        addon.inner.id
    )
    .fetch_optional(&mut txn.executor())
    .await?;
    if owner.is_some_and(|owner| owner != set_id) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "addon belongs to another addon set",
            "",
        ));
    }
    // Once an addon has been sold, only its name may change: purchases snapshot the selection by
    // option index and the bookkeeping split, so their meaning must not move.
    let has_purchases = sqlx::query_scalar!(
        r#"select exists (
            select 1 from purchased_product_addons where addon_id = $1
        ) as "exists!""#,
        addon.inner.id
    )
    .fetch_one(&mut txn.executor())
    .await?;
    if has_purchases {
        let existing = product::load_addons(txn, set_id, None).await?;
        let unchanged_structure = existing
            .iter()
            .find(|old| old.inner.id == addon.inner.id)
            .is_some_and(|old| {
                old.inner.multiple_alternatives == addon.inner.multiple_alternatives
                    && old.inner.has_text_field == addon.inner.has_text_field
                    && old.inner.required == addon.inner.required
                    && old.options.len() == addon.options.len()
                    && old.options.iter().zip(&addon.options).all(|(old, new)| {
                        old.id == new.id
                            && old.idx == new.idx
                            && old.price == new.price
                            && old.name == new.name
                            && old.bookkeeping_prices == new.bookkeeping_prices
                            && old.bookkeeping_price_categories == new.bookkeeping_price_categories
                    })
            });
        if !unchanged_structure {
            return Err(MinilithEndpointError::bad_frontend_code(
                "only the addon name may change after it has been purchased",
                "",
            ));
        }
        sqlx::query!(
            "update product_addons set name = $2 where id = $1",
            addon.inner.id,
            addon.inner.name.to_json_value()
        )
        .execute(&mut txn.executor())
        .await?;
        return Ok(());
    }
    sqlx::query!(r#"insert into product_addons (id, set_id, idx, name, multiple_alternatives, has_text_field, required)
        values ($1, $2, coalesce((select max(idx) + 1 from product_addons where set_id = $2), 0), $3, $4, $5, $6)
        on conflict (id) do update set name = excluded.name, multiple_alternatives = excluded.multiple_alternatives,
            has_text_field = excluded.has_text_field, required = excluded.required"#,
        addon.inner.id, set_id, addon.inner.name.to_json_value(), addon.inner.multiple_alternatives,
        addon.inner.has_text_field, addon.inner.required).execute(&mut txn.executor()).await?;
    sqlx::query!(
        "delete from product_addon_options where product_addon_id = $1",
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
        sqlx::query!(
            r#"insert into product_addon_options (id, product_addon_id, idx, name, price)
            values ($1, $2, $3, $4, $5)"#,
            option.id,
            addon.inner.id,
            index,
            option.name.to_json_value(),
            PgMoney(option.price)
        )
        .execute(&mut txn.executor())
        .await?;
        write_addon_bookkeeping(
            &mut *txn,
            option.id,
            &prices,
            &option.bookkeeping_price_categories,
        )
        .await?;
    }
    Ok(())
}

/// Splits an option's price over the given bookkeeping categories (matched by their english tag
/// name). Any previous split is replaced.
async fn write_addon_bookkeeping(
    txn: &mut Transaction<'_>,
    option_id: Uuid,
    prices: &[PgMoney],
    categories: &[String],
) -> MinilithResult<()> {
    sqlx::query!(
        "delete from product_addon_option_bookkeepings where option_id = $1",
        option_id
    )
    .execute(&mut txn.executor())
    .await?;
    let inserted = sqlx::query!(
        r#"insert into product_addon_option_bookkeepings (id, option_id, price, category)
        select uuidv4(), $1, entry.price, tag.id
        from unnest($2::money[], $3::text[]) as entry(price, category)
        inner join product_tags tag on tag.name->>'en' = entry.category"#,
        option_id,
        prices,
        categories,
    )
    .execute(&mut txn.executor())
    .await?
    .rows_affected();
    // An unknown category would otherwise be dropped silently and the option's split would no
    // longer add up to its price.
    if usize::try_from(inserted).ok() != Some(prices.len()) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "unknown bookkeeping category",
            "bookkeeping_price_categories",
        ));
    }
    Ok(())
}
