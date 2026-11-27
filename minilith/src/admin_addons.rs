use bin_common::Transaction;
use sqlx::postgres::types::PgMoney;
use uuid::Uuid;

use crate::{
    MinilithEndpointError, MinilithResult,
    product::{self, AvailableAddon},
};

/// Writes a product's mandatory base-price add-on: a hidden, required add-on with one option named
/// after the product, priced at `price`, whose bookkeeping split is `prices` over `bookkeeping_tags`
/// (both may be empty, which books nothing).
///
/// Purchases snapshot their prices, so this stays editable after the first sale.
///
/// # Errors
///
/// `DB` if the set, add-on, option or bookkeeping rows cannot be written.
pub async fn save_base_price_addon(
    txn: &mut Transaction<'_>,
    product_id: Uuid,
    product_name: &crate::InternationalizedString,
    price: i64,
    bookkeeping_tags: &[Uuid],
    prices: &[PgMoney],
) -> MinilithResult<()> {
    if bookkeeping_tags.len() != prices.len() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "bookkeeping tags and prices must have the same length",
            "bookkeeping_tag_ids",
        ));
    }
    sqlx::query!(
        r#"insert into product_addon_sets (id, product_id) values ($1, $1)
        on conflict (id) do nothing"#,
        product_id
    )
    .execute(&mut txn.executor())
    .await?;
    let addon_id = match sqlx::query_scalar!(
        "select id from product_addons where set_id = $1 and is_base_price",
        product_id
    )
    .fetch_optional(&mut txn.executor())
    .await?
    {
        Some(addon_id) => addon_id,
        None => {
            sqlx::query_scalar!(
                r#"insert into product_addons
            (id, set_id, idx, name, multiple_alternatives, has_text_field, required, is_base_price)
            values (uuidv4(), $1, -1, '{"en":"Price","sv":"Pris"}'::jsonb, false, false, true, true)
            returning id"#,
                product_id
            )
            .fetch_one(&mut txn.executor())
            .await?
        }
    };
    sqlx::query!(
        "insert into product_addon_links (product_id, addon_id) values ($1, $2)
        on conflict do nothing",
        product_id,
        addon_id
    )
    .execute(&mut txn.executor())
    .await?;
    let option_id = match sqlx::query_scalar!(
        "select id from product_addon_options where product_addon_id = $1 and idx = 0",
        addon_id
    )
    .fetch_optional(&mut txn.executor())
    .await?
    {
        Some(option_id) => {
            sqlx::query!(
                "update product_addon_options set name = $2, price = $3 where id = $1",
                option_id,
                product_name.to_json_value(),
                PgMoney(price),
            )
            .execute(&mut txn.executor())
            .await?;
            option_id
        }
        None => {
            sqlx::query_scalar!(
                "insert into product_addon_options (id, product_addon_id, idx, name, price)
            values (uuidv4(), $1, 0, $2, $3)
            returning id",
                addon_id,
                product_name.to_json_value(),
                PgMoney(price),
            )
            .fetch_one(&mut txn.executor())
            .await?
        }
    };
    sqlx::query!(
        "delete from product_addon_option_bookkeepings where option_id = $1",
        option_id
    )
    .execute(&mut txn.executor())
    .await?;
    for (tag_id, price) in bookkeeping_tags.iter().zip(prices) {
        sqlx::query!(
            r#"insert into product_addon_option_bookkeepings (id, option_id, price, category)
            values (uuidv4(), $1, $2, $3)"#,
            option_id,
            price,
            tag_id
        )
        .execute(&mut txn.executor())
        .await?;
    }
    Ok(())
}

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
        let existing = product::load_addons(txn, set_id, None, false).await?;
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
