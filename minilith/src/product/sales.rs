//! Counter sales paid on a `SumUp` reader, with or without a registered sales device.
//!
//! These sales do not use the reservation/queue flow: the customer is standing at the counter, so
//! there is nothing to hold back from other buyers for minutes at a time. Instead the chosen lines
//! are stored as a pending [`physical_sales`] entry (which reserves `sales_stock`) and are turned
//! into real purchases by the provider callback.
//!
//! The same `begin` is used by the regular app purchase endpoint when `reader_id` is set and by
//! `POST /v1/sales-device/purchases` for multi-item counter sales.

use bin_common::Transaction;
use minilith_errors::{MinilithErrorOptionExt as _, MinilithErrorResultExt as _};
use poem_openapi::{Enum, Object};
use sqlx::PgExecutor;
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use super::models::{BoughtAddon, PurchaseProvider};
use super::{access, ensure_affected_rows, purchase};
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, MinilithEndpointError, MinilithResult,
    transactions,
};

/// Purchases made at a counter have no customer account. They are owned by this sentinel user.
pub(super) const ANONYMOUS_BUYER: &str = "internal:anonymous-physical-sales";

/// A registered sales device, as far as purchasing is concerned.
#[derive(Debug, Clone, Copy)]
pub(super) struct DeviceScope {
    pub id: Uuid,
    pub group_id: Option<Uuid>,
    pub activity_id: Option<Uuid>,
}

/// Who is buying, and what they are allowed to sell/buy.
#[derive(Debug, Clone, Copy)]
pub(super) enum Viewer<'a> {
    User(&'a str),
    Device(&'a DeviceScope),
}

/// One line of a counter sale.
pub(super) struct Item {
    pub product_id: Uuid,
    pub addons: Vec<BoughtAddon>,
}

/// Reads a device from its group/activity. Expired devices are rejected.
///
/// # Errors
///
/// NOT FOUND when the id does not exist or has expired.
pub(super) async fn load_device(db: impl PgExecutor<'_>, id: Uuid) -> MinilithResult<DeviceScope> {
    let row = sqlx::query!(
        "select group_id, activity_id from physical_sales_clients
        where id = $1 and until > now()",
        id,
    )
    .fetch_optional(db)
    .await?
    .wrap_err_not_found()?;
    Ok(DeviceScope {
        id,
        group_id: row.group_id,
        activity_id: row.activity_id,
    })
}

/// Readers a device may use, matching `GET /v1/readers` for the same scope.
///
/// # Errors
///
/// DB errors.
pub(super) async fn reader_group_ids(
    db: &mut Transaction<'_>,
    device: &DeviceScope,
) -> MinilithResult<Vec<Uuid>> {
    if let Some(activity_id) = device.activity_id {
        let group_ids = sqlx::query_scalar!(
            r#"select group_id from activity_hosts where activity_id = $1
            union
            select target.group_id
                from activities
                inner join sales_targets target on target.id = activities.sales_target_id
                where activities.id = $1
            union
            select stock_group from activities
                where id = $1 and stock_group is not null"#,
            activity_id,
        )
        .fetch_all(&mut db.executor())
        .await?
        .into_iter()
        .flatten()
        .collect();
        Ok(group_ids)
    } else {
        Ok(device.group_id.into_iter().collect())
    }
}

/// Makes sure the reader is paired with a group the device may charge on.
///
/// # Errors
///
/// Rejects readers outside the device's activity hosts/sales target/stock group or group.
pub(super) async fn ensure_reader_allowed(
    db: &mut Transaction<'_>,
    device: &DeviceScope,
    reader_id: &str,
) -> MinilithResult<()> {
    let group_ids = reader_group_ids(db, device).await?;
    let allowed = sqlx::query_scalar!(
        r#"select exists (
            select 1 from physical_payment_terminals
            where id = $1 and group_id = any($2)
        ) as "exists!""#,
        reader_id,
        &group_ids,
    )
    .fetch_one(&mut db.executor())
    .await?;
    if allowed {
        Ok(())
    } else {
        Err(MinilithEndpointError::bad_frontend_code(
            "this reader does not belong to the sales device",
            "",
        ))
    }
}

/// Starts one transaction for all `items` and pays it on `reader_id` (or for free when the total
/// is zero). Returns the transactions id. The purchases are created by the provider callback.
///
/// # Errors
///
/// AUTH, DB, or the provider rejecting the payment.
#[allow(
    clippy::too_many_lines,
    reason = "keeps the stock claim, payment and cleanup of one counter sale visible"
)]
pub(super) async fn begin(
    ctx: &ContextWrapper,
    viewer: Viewer<'_>,
    buyer_id: &str,
    mut items: Vec<Item>,
    reader_id: Option<&str>,
) -> MinilithResult<Uuid> {
    if items.is_empty() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "a sale needs at least one item",
            "",
        ));
    }
    if items.len() > 100 {
        return Err(MinilithEndpointError::bad_frontend_code(
            "too many items in one sale",
            "",
        ));
    }
    // Lock products in a stable order so two multi-item sales cannot deadlock.
    items.sort_unstable_by_key(|item| item.product_id);

    let transaction_id = Uuid::new_v4();
    let mut txn = ctx.db.begin().await?;
    ensure_buyer_exists(ctx, &mut txn, buyer_id).await?;

    let language = sqlx::query_scalar!("select language from users where id = $1", buyer_id)
        .fetch_one(&mut txn.executor())
        .await?;
    let language = ctx
        .decrypt_string(language)
        .unwrap_or_else(|| "en".to_owned());

    sqlx::query!(
        "insert into physical_sales (transaction_id, sales_device_id, buyer_id, total)
        values ($1, $2, $3, '0.00'::money)",
        transaction_id,
        viewer.device().map(|device| device.id),
        buyer_id,
    )
    .execute(&mut txn.executor())
    .await?;

    let mut wares = Vec::new();
    for item in &mut items {
        let product = sqlx::query!(
            r#"select
                product.name as "name!: DIS",
                base.price as "price!",
                (now() between product.sales_start and product.sales_stop) as "on_sale!",
                (select assignment.activity_id from product_activity_assignments assignment
                    where assignment.product_id = product.id
                    order by assignment.activity_id limit 1) as "activity_id?"
            from products product
            inner join product_base_prices base on base.product_id = product.id
            where product.id = $1
            and product.kind <> 'activity_visibility'
            and (
                -- a sales device may only sell what it is registered for
                ($2::uuid is null and $3::uuid is null)
                or product.store_general_sales and (
                    ($2::uuid is not null and (
                        product.group_id = $2
                        or exists (
                            select 1
                            from product_activity_assignments assignment
                            inner join activity_hosts host
                                on host.activity_id = assignment.activity_id
                            where assignment.product_id = product.id
                            and host.group_id = $2
                        )
                    ))
                    or ($3::uuid is not null and exists (
                        select 1 from product_activity_assignments assignment
                        where assignment.product_id = product.id
                        and assignment.activity_id = $3
                    ))
                )
            )"#,
            item.product_id,
            viewer.device_group(),
            viewer.device_activity(),
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .wrap_err_bad_frontend("this product cannot be sold by this device")?;
        if !product.on_sale {
            return Err(MinilithEndpointError::bad_frontend_code(
                "the product is not for sale right now",
                item.product_id,
            ));
        }
        if let Viewer::User(user_id) = viewer {
            access::ensure_user_may_purchase_ticket(&mut txn.executor(), user_id, item.product_id)
                .await?;
        }
        purchase::validate_addons(&mut txn, &mut item.addons, item.product_id).await?;

        // Claim one unit of stock. A null `sales_stock` means unlimited stock.
        let claimed = sqlx::query!(
            "update products
            set sales_stock = sales_stock - 1
            where id = $1 and (sales_stock is null or sales_stock > 0)",
            item.product_id,
        )
        .execute(&mut txn.executor())
        .await?;
        if claimed.rows_affected() != 1 {
            return Err(MinilithEndpointError::bad_user_input(
                "item",
                "",
                "the product is sold out",
                "items",
            ));
        }

        let activity_id = viewer.device_activity().or(product.activity_id);
        let item_id = sqlx::query_scalar!(
            "insert into physical_sale_items
                (transaction_id, product_id, product_price, activity_id)
            values ($1, $2, $3, $4)
            returning id",
            transaction_id,
            item.product_id,
            product.price,
            activity_id,
        )
        .fetch_one(&mut txn.executor())
        .await?;

        for addon in &item.addons {
            sqlx::query!(
                "insert into physical_sale_item_addons
                    (item_id, addon_id, selected_options, selected_options_prices, selected_text)
                values ($1, $2, $3,
                    (select coalesce(array_agg(coalesce(opt.price, 0::money) order by chosen.ord), '{}'::money[])
                        from unnest($3::integer[]) with ordinality as chosen(idx, ord)
                        left join product_addon_options opt
                            on opt.product_addon_id = $2 and opt.idx = chosen.idx),
                    $4)",
                item_id,
                addon.id,
                addon.selected_options.as_deref().unwrap_or(&[]),
                addon.selected_text.as_deref().unwrap_or(""),
            )
            .execute(&mut txn.executor())
            .await?;
        }

        let name = product.name.resolve_intl(&language, "<product>");
        let base_name = if let Some(activity_id) = activity_id {
            let title = sqlx::query_scalar!(
                "select title as \"title!: DIS\" from activities where id = $1",
                activity_id,
            )
            .fetch_one(&mut txn.executor())
            .await?;
            format!("{} - {name}", title.resolve_intl(&language, "<activity>"))
        } else {
            name.to_owned()
        };
        wares.push(transactions::Ware {
            name: base_name,
            amount: product.price.0,
            tax: 1.0,
            currency: transactions::Currency::Sek,
        });
        let options = sqlx::query!(
            r#"select
                addon.name as "addon_name!: DIS",
                option.name as "option_name!: DIS",
                option.price as "price!"
            from physical_sale_item_addons item_addon
            inner join product_addons addon on addon.id = item_addon.addon_id
            cross join lateral unnest(item_addon.selected_options) with ordinality
                as chosen(idx, ord)
            inner join product_addon_options option
                on option.product_addon_id = addon.id and option.idx = chosen.idx
            where item_addon.item_id = $1
            order by addon.idx, chosen.ord"#,
            item_id,
        )
        .fetch_all(&mut txn.executor())
        .await?;
        for option in options {
            wares.push(transactions::Ware {
                name: format!(
                    "    {} - {}",
                    option.addon_name.resolve_intl(&language, "<addon>"),
                    option.option_name.resolve_intl(&language, "<option>"),
                ),
                amount: option.price.0,
                tax: 1.0,
                currency: transactions::Currency::Sek,
            });
        }
    }

    let total = wares.iter().map(|ware| ware.amount).sum::<i64>();
    sqlx::query!(
        "update physical_sales set total = $2::money where transaction_id = $1",
        transaction_id,
        sqlx::postgres::types::PgMoney(total),
    )
    .execute(&mut txn.executor())
    .await?;

    let provider = if total == 0 {
        PurchaseProvider::Free
    } else {
        let Some(reader_id) = reader_id else {
            return Err(MinilithEndpointError::bad_frontend_code(
                "a paid sale needs a reader",
                "",
            ));
        };
        if let Viewer::Device(device) = viewer {
            ensure_reader_allowed(&mut txn, device, reader_id).await?;
        }
        PurchaseProvider::Sumup
    };
    let reader_id = (provider == PurchaseProvider::Sumup)
        .then(|| reader_id.map(ToOwned::to_owned))
        .flatten();

    let timeout = (OffsetDateTime::now_utc() + time::Duration::minutes(20))
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .wrap_err_internal("failed to format the sale timeout")?;
    let payment = transactions::CreatePaymentRequest {
        id: transaction_id,
        customer_id: Some(buyer_id.to_owned()),
        timeout,
        wares,
        redirect_url: None,
        reader_id,
    };
    txn.commit().await?;

    if let Err(error) = purchase::send_payment(ctx, provider, payment).await {
        // Never leave stock claimed for a payment the provider refused.
        if let Ok(mut cleanup_txn) = ctx.db.begin().await {
            if let Err(cleanup) = cancel(&mut cleanup_txn, transaction_id).await {
                tracing::error!(?cleanup, "failed to release a refused counter sale");
            } else if let Err(commit) = cleanup_txn.commit().await {
                tracing::error!(?commit, "failed to commit a released counter sale");
            }
        }
        return Err(error);
    }
    Ok(transaction_id)
}

impl<'a> Viewer<'a> {
    fn device(self) -> Option<&'a DeviceScope> {
        match self {
            Viewer::User(_) => None,
            Viewer::Device(device) => Some(device),
        }
    }
    fn device_group(self) -> Option<Uuid> {
        self.device().and_then(|device| device.group_id)
    }
    fn device_activity(self) -> Option<Uuid> {
        self.device().and_then(|device| device.activity_id)
    }
}

/// Makes sure the anonymous counter buyer exists so purchases can reference it.
async fn ensure_buyer_exists(
    ctx: &ContextWrapper,
    txn: &mut Transaction<'_>,
    buyer_id: &str,
) -> MinilithResult<()> {
    if buyer_id != ANONYMOUS_BUYER {
        let exists = sqlx::query_scalar!(
            "select exists (select 1 from users where id = $1) as \"exists!\"",
            buyer_id
        )
        .fetch_one(&mut txn.executor())
        .await?;
        if !exists {
            return Err(MinilithEndpointError::bad_user_input(
                "buyer_id",
                "",
                "unknown buyer",
                "buyer_id",
            ));
        }
        return Ok(());
    }
    // The name and language are encrypted with the runtime key, so the row cannot live in a
    // migration. Insert it lazily; this is idempotent.
    sqlx::query!(
        "insert into users (id, name, language) values ($1, $2, $3)
        on conflict (id) do nothing",
        ANONYMOUS_BUYER,
        ctx.encrypt("Anonymous counter sale"),
        ctx.encrypt("en"),
    )
    .execute(&mut txn.executor())
    .await?;
    Ok(())
}

/// Turns the paid pending lines into purchases. Returns `false` when the transaction is not a
/// counter sale. Idempotent: a repeated callback finds the header but no items.
///
/// # Errors
///
/// DB errors.
#[allow(
    clippy::too_many_lines,
    reason = "keeps the pending-sale snapshot and the created purchases auditable in one place"
)]
pub(super) async fn complete(
    txn: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<bool> {
    let Some(header) = sqlx::query!(
        "select buyer_id from physical_sales where transaction_id = $1",
        transaction_id,
    )
    .fetch_optional(&mut txn.executor())
    .await?
    else {
        return Ok(false);
    };
    let items = sqlx::query!(
        "select id, product_id, product_price, activity_id
        from physical_sale_items
        where transaction_id = $1
        for update",
        transaction_id,
    )
    .fetch_all(&mut txn.executor())
    .await?;
    if items.is_empty() {
        // Either an already processed callback, or the payment succeeded after the sale was
        // cancelled. The first is fine; the second needs a human.
        let already_completed = sqlx::query_scalar!(
            "select exists (
                select 1 from purchase_transactions where transaction_id = $1
            ) as \"exists!\"",
            transaction_id,
        )
        .fetch_one(&mut txn.executor())
        .await?;
        if !already_completed {
            minilith_errors::alert(
                minilith_errors::AlertLevel::L1,
                "a counter sale was paid after it was cancelled",
            );
            tracing::error!(%transaction_id, "a counter sale was paid after it was cancelled");
        }
        return Ok(true);
    }
    for item in &items {
        sqlx::query!(
            "update products set been_purchased = true where id = $1",
            item.product_id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "insert into purchases
                (id, purchaser_id, owner_id, product_id, paid_price, activity_id, purchased_at)
            values ($1, $2, $2, $3,
                $4 + coalesce((
                    select sum(price)
                    from physical_sale_item_addons addon
                    cross join lateral unnest(addon.selected_options_prices) price
                    where addon.item_id = $1
                ), '0.00'::money),
                $5, now())",
            item.id,
            header.buyer_id,
            item.product_id,
            item.product_price,
            item.activity_id,
        )
        .execute(&mut txn.executor())
        .await?;
        // The base price is one of the add-ons, so it belongs to the purchase's add-on snapshot.
        sqlx::query!(
            r#"insert into purchased_product_addons
            (addon_id, purchase_id, selected_options, selected_options_prices, selected_text)
            select addon.id, $1, '{0}'::integer[], array[$2::money], ''
            from product_addons addon
            where addon.set_id = $3 and addon.is_base_price"#,
            item.id,
            item.product_price,
            item.product_id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "insert into purchased_product_addons
            (addon_id, purchase_id, selected_options, selected_options_prices, selected_text)
            select addon_id, item_id, selected_options, selected_options_prices, selected_text
            from physical_sale_item_addons
            where item_id = $1",
            item.id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "insert into purchase_transactions
            (purchase_id, seller_id, buyer_id, completed_at, transaction_id)
            values ($1, null, $2, now(), $3)",
            item.id,
            header.buyer_id,
            transaction_id,
        )
        .execute(&mut txn.executor())
        .await?;
    }
    // Deleting the lines cascades their addon snapshots. The header stays for status/audit.
    let removed = sqlx::query!(
        "delete from physical_sale_items where transaction_id = $1",
        transaction_id,
    )
    .execute(&mut txn.executor())
    .await?;
    ensure_affected_rows(
        removed.rows_affected(),
        items.len(),
        "counter sale items changed while completing",
    )?;
    Ok(true)
}

/// Releases the stock of a cancelled/refused counter sale and drops its lines. Returns `false` when
/// there was nothing pending for this transaction.
///
/// # Errors
///
/// DB errors.
pub(super) async fn cancel(
    txn: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<bool> {
    let items = sqlx::query!(
        "select product_id from physical_sale_items
        where transaction_id = $1
        for update",
        transaction_id,
    )
    .fetch_all(&mut txn.executor())
    .await?;
    if items.is_empty() {
        return Ok(false);
    }
    for item in &items {
        sqlx::query!(
            "update products set sales_stock = sales_stock + 1
            where id = $1 and sales_stock is not null",
            item.product_id,
        )
        .execute(&mut txn.executor())
        .await?;
    }
    sqlx::query!(
        "delete from physical_sale_items where transaction_id = $1",
        transaction_id,
    )
    .execute(&mut txn.executor())
    .await?;
    Ok(true)
}

#[derive(Debug, Clone, Object)]
pub struct SaleStatus {
    pub state: SaleState,
    pub total: i64,
    pub purchase_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, Enum)]
#[oai(rename_all = "lowercase")]
pub enum SaleState {
    Pending,
    Paid,
    Refunded,
    Cancelled,
    Unknown,
}

/// # Errors
///
/// NOT FOUND when the transaction was not started by this device.
pub(super) async fn status(
    ctx: &ContextWrapper,
    device: &DeviceScope,
    transaction_id: Uuid,
) -> MinilithResult<SaleStatus> {
    let header = sqlx::query!(
        "select total from physical_sales
        where transaction_id = $1 and sales_device_id = $2",
        transaction_id,
        device.id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;
    let info = ctx
        .transactions_post("/v0/info")
        .json(&transactions::InfoRequest {
            transaction_ids: vec![transaction_id],
        })
        .send()
        .await
        .wrap_err_internal("failed to read the sale transaction")?
        .error_for_status()
        .wrap_err_internal("failed to read the sale transaction")?
        .json::<Vec<transactions::SingleInfoResponse>>()
        .await
        .wrap_err_internal("failed to parse the sale transaction")?
        .into_iter()
        .next();
    let state = match info.map(|info| info.state) {
        Some(fed_auth_verifier::callbacks::TransactionState::Pending) => SaleState::Pending,
        Some(fed_auth_verifier::callbacks::TransactionState::Paid) => SaleState::Paid,
        Some(fed_auth_verifier::callbacks::TransactionState::Refunded) => SaleState::Refunded,
        Some(fed_auth_verifier::callbacks::TransactionState::Cancelled) => SaleState::Cancelled,
        None => SaleState::Unknown,
    };
    let purchase_ids = sqlx::query_scalar!(
        "select purchase_id from purchase_transactions where transaction_id = $1",
        transaction_id,
    )
    .fetch_all(&ctx.db)
    .await?;
    Ok(SaleStatus {
        state,
        total: header.total.0,
        purchase_ids,
    })
}
