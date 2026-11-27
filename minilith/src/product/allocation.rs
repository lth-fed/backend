use std::collections::HashSet;
use std::ops::ControlFlow;

use bin_common::{PgPool, Transaction};
use rand::random;
use sqlx::PgExecutor;
use sqlx::postgres::types::PgInterval;
use uuid::Uuid;

use super::{
    ensure_affected_rows,
    flow::{unlist_user_purchase_flow, unlist_users_purchase_flow},
    notifications::{TicketNotification, notify_ticket_users},
};
use crate::{ContextWrapper, MinilithResult};

/// # Panics
///
/// Never.
pub(super) fn new_timeout_interval() -> PgInterval {
    let minute_in_microseconds = 1_000_000_f64 * 60_f64;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "it's not gonna be big enough"
    )]
    PgInterval {
        months: 0,
        days: 0,
        microseconds: (minute_in_microseconds * (15. + 3. * random::<f64>())) as i64,
    }
}

/// Guaranteed invitees whose copy is not taken yet: they have neither a purchase nor a live
/// reservation, so their unit still has to be held back from public sales.
pub(super) async fn unreserved_invites(
    db: impl PgExecutor<'_>,
    product_id: Uuid,
) -> MinilithResult<i32> {
    let count = sqlx::query_scalar!(
        r#"select coalesce(count(*), 0)::int as "count!"
        from product_buyers buyer
        where buyer.product_id = $1
        and buyer.guaranteed
        and not exists (
            select 1 from purchases purchase
            where purchase.product_id = $1
            and purchase.owner_id = buyer.user_id
            and purchase.refunded_id is null
        )
        and not exists (
            select 1 from product_reservations reservation
            where reservation.product_id = $1
            and reservation.user_id = buyer.user_id
        )"#,
        product_id,
    )
    .fetch_one(db)
    .await?;
    Ok(count)
}

/// Users that are guaranteed a copy of this product.
pub(super) async fn guaranteed_invitees(
    db: impl PgExecutor<'_>,
    product_id: Uuid,
) -> MinilithResult<HashSet<String>> {
    let invitees = sqlx::query_scalar!(
        "select user_id from product_buyers where product_id = $1 and guaranteed",
        product_id,
    )
    .fetch_all(db)
    .await?;
    Ok(invitees.into_iter().collect())
}

/// Whether this user may buy the product outside its groups.
pub(super) async fn is_invited_buyer(
    db: impl PgExecutor<'_>,
    product_id: Uuid,
    user_id: &str,
) -> MinilithResult<bool> {
    let invited = sqlx::query_scalar!(
        r#"select exists (
            select 1 from product_buyers where product_id = $1 and user_id = $2
        ) as "exists!""#,
        product_id,
        user_id,
    )
    .fetch_one(db)
    .await?;
    Ok(invited)
}

/// The product whose reservation queue the waiters of `product_id` belong to: the activity's base
/// ticket. Only a base ticket keeps a reservation queue; everybody else is queued there and keeps
/// the kind they want in `product_reservation_queuers.original_product_id`.
pub(super) async fn queue_base(db: impl PgExecutor<'_>, product_id: Uuid) -> MinilithResult<Uuid> {
    let base = sqlx::query_scalar!(
        r#"select coalesce((
            select activity.base_ticket
            from products product
            inner join product_activity_assignments assignment
                on assignment.product_id = product.id
            inner join activities activity on activity.id = assignment.activity_id
            where product.id = $1 and product.kind = 'ticket'
            order by activity.id
            limit 1
        ), $1) as "base!""#,
        product_id
    )
    .fetch_one(db)
    .await?;
    Ok(base)
}

/// Atomically reserves up to `requested` places from `sales_stock`. A null stock means the
/// product is only sold physically, so it can never be reserved.
///
/// `hold_back_invites` keeps the units promised to guaranteed invitees out of the public pool.
/// Callers that already added the invited buyers to `requested` pass `false` instead, which is
/// what the bulk release/promotion paths do to give invitees their own unit first.
pub(super) async fn reserve_ticket_capacity(
    db: &mut Transaction<'_>,
    ticket_kind: Uuid,
    requested: i32,
    hold_back_invites: bool,
) -> MinilithResult<i32> {
    // Taking from `sales_stock` atomically is the reservation itself.
    let granted = sqlx::query_scalar!(
        r#"with promised as (
            select coalesce(count(*), 0)::int as remaining
            from product_buyers buyer
            where buyer.product_id = $1
            and buyer.guaranteed
            and not exists (
                select 1 from purchases purchase
                where purchase.product_id = $1
                and purchase.owner_id = buyer.user_id
                and purchase.refunded_id is null
            )
            and not exists (
                select 1 from product_reservations reservation
                where reservation.product_id = $1
                and reservation.user_id = buyer.user_id
            )
        ), capacity as materialized (
            select kind.id,
                least(
                    $2,
                    greatest(
                        0,
                        kind.sales_stock - case when $3 then promised.remaining else 0 end
                    )
                ) as granted
            from products kind
            cross join promised
            where kind.id = $1 and kind.sales_stock is not null
            for update of kind
        ), updated as (
            update products
            set sales_stock = sales_stock - capacity.granted
            from capacity
            where products.id = capacity.id
            returning products.id
        )
        select capacity.granted::int as "granted!"
        from capacity"#,
        ticket_kind,
        requested,
        hold_back_invites,
    )
    .fetch_optional(&mut db.executor())
    .await?
    .unwrap_or(0);
    Ok(granted)
}

/// How many reservations the public pool still allows, plus how many invited buyers are in this
/// batch. Used by the bulk paths to size a request that gives invitees their reserved unit first.
pub(super) async fn public_capacity(
    db: &mut Transaction<'_>,
    ticket_kind: Uuid,
) -> MinilithResult<i32> {
    let stock = sqlx::query_scalar!(
        r#"select coalesce(sales_stock, 0) as "stock!" from products where id = $1"#,
        ticket_kind,
    )
    .fetch_one(&mut db.executor())
    .await?;
    let promised = unreserved_invites(&mut db.executor(), ticket_kind).await?;
    Ok((stock - promised).max(0))
}

/// Checks for reservations which are timed out. If any is found, it's removed.
/// Call [`give_reservations`] after calling this.
///
/// # Errors
///
/// Failures from cancelling transactions.
pub(super) async fn remove_reservation(db: &PgPool) -> MinilithResult<ControlFlow<()>> {
    let mut txn = db.begin().await?;
    // Lock the flow first; child reservation rows are always locked second.
    let removed_reservation = sqlx::query!(
        "select flow.user_id, flow.product_id
        from users_in_purchase_flow flow
        inner join product_reservations reservation
            on reservation.user_id = flow.user_id
            and reservation.product_id = flow.product_id
        where reservation.timeout < now()
        and reservation.transaction_id is null
        and flow.reservation = flow.user_id
        order by flow.user_id
        limit 1
        for update of flow skip locked",
    )
    .fetch_optional(&mut txn.executor())
    .await?;
    let do_continue = removed_reservation.is_some();
    if let Some(reservation) = removed_reservation {
        let affected = sqlx::query!(
            "delete from product_reservations
            where user_id = $1
            and product_id = $2
            and timeout < now()
            and transaction_id is null",
            reservation.user_id,
            reservation.product_id,
        )
        .execute(&mut txn.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            1,
            "expired reservation changed after its flow was locked",
        )?;
        sqlx::query!(
            "update products
            set sales_stock = sales_stock + 1
            where id = $1 and sales_stock is not null",
            reservation.product_id,
        )
        .execute(&mut txn.executor())
        .await?;
        unlist_user_purchase_flow(&mut txn, &reservation.user_id).await?;
    }
    txn.commit().await?;
    Ok(if do_continue {
        ControlFlow::Continue(())
    } else {
        ControlFlow::Break(())
    })
}
/// Reserves stock for as many of the first `queuers` as `kind` can still fit and returns them.
/// Guaranteed invitees go first: [`public_capacity`] holds their promised units back from the
/// public pool.
async fn pick_reservations(
    db: &mut Transaction<'_>,
    kind: Uuid,
    mut queuers: Vec<String>,
) -> MinilithResult<Vec<String>> {
    let invitees = guaranteed_invitees(&mut db.executor(), kind).await?;
    queuers.sort_by_key(|user| !invitees.contains(user));
    let invited_here = queuers
        .iter()
        .filter(|user| invitees.contains(*user))
        .count();
    let public = public_capacity(db, kind).await?;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we'll never get this high"
    )]
    let requested = (queuers.len() as i32).min(public + invited_here as i32);
    let granted = reserve_ticket_capacity(db, kind, requested, false).await?;
    #[allow(
        clippy::cast_sign_loss,
        reason = "granted is capped by the queue length"
    )]
    queuers.truncate(granted as usize);
    Ok(queuers)
}

/// Moves queuers that were pushed down to their activity's base ticket into the kind they
/// originally wanted. Their flow has to point at `kind` before any of its child rows are written,
/// because the flow → queue foreign keys are immediate.
async fn pull_up(db: &mut Transaction<'_>, kind: Uuid, pulled: &[String]) -> MinilithResult<()> {
    if pulled.is_empty() {
        return Ok(());
    }
    let affected = sqlx::query!(
        "delete from product_reservation_queuers
        where user_id = any($1)",
        pulled
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        pulled.len(),
        "failed to remove the pulled-up users from the fallback queue",
    )?;
    let affected = sqlx::query!(
        "update users_in_purchase_flow
        set product_id = $2
        where user_id = any($1) and product_id <> $2 and reservation_queue = user_id",
        pulled,
        kind
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        pulled.len(),
        "failed to move the pulled-up users to their original ticket kind",
    )?;
    Ok(())
}
/// If there are reservation spots left, a person from the `reservation_queue` will get a reservation.
///
/// Also handles the case where there's a stray in the reservation queue.
/// Returns promoted users for push delivery after the caller commits.
///
/// # Errors
///
/// None:) only db.
pub(super) async fn give_reservations(
    ticket_kind: Uuid,
    fetch_n: i32,
    db: &mut Transaction<'_>,
) -> MinilithResult<Vec<String>> {
    // Queuers of the kind itself plus the people that were pushed down to their activity's base
    // ticket and originally queued for this kind.
    let candidates = sqlx::query!(
        r#"select flow.user_id, queuer.product_id as "queued_kind!"
        from users_in_purchase_flow flow
        inner join product_reservation_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.product_id = flow.product_id
        where flow.reservation_queue = flow.user_id
        -- queued in this kind (only base tickets are queue kinds) or waiting for it as the
        -- kind they originally wanted
        and (queuer.product_id = $1 or queuer.original_product_id = $1)
        order by queuer.placement asc, flow.user_id
        limit $2
        for update of flow skip locked"#,
        ticket_kind,
        i64::from(fetch_n)
    )
    .fetch_all(&mut db.executor())
    .await?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let users: Vec<String> = candidates.iter().map(|row| row.user_id.clone()).collect();
    let new_reservations = pick_reservations(db, ticket_kind, users).await?;
    if new_reservations.is_empty() {
        return Ok(Vec::new());
    }

    let pulled: Vec<String> = candidates
        .iter()
        .filter(|row| row.queued_kind != ticket_kind && new_reservations.contains(&row.user_id))
        .map(|row| row.user_id.clone())
        .collect();
    pull_up(db, ticket_kind, &pulled).await?;
    let timestamps: Vec<PgInterval> = std::iter::repeat_with(new_timeout_interval)
        .take(new_reservations.len())
        .collect();
    let affected = sqlx::query!(
        "insert into product_reservations
            (user_id, product_id, product_price, transaction_id, timeout)
        select t.user_id, $2, base.price, null, (t.from_now + now())
        from unnest($1::text[], $3::interval[]) as t(user_id, from_now)
        inner join products kind on kind.id = $2
        inner join product_base_prices base on base.product_id = kind.id",
        &new_reservations,
        ticket_kind,
        &timestamps
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        new_reservations.len(),
        "failed to create promoted ticket reservations",
    )?;
    let affected = sqlx::query!(
        "update users_in_purchase_flow
            set reservation_queue = null, reservation = user_id
        where user_id = any($1)
        and product_id = $2
        and reservation_queue = user_id
        and release_queue is null
        and reservation is null",
        &new_reservations,
        ticket_kind,
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        new_reservations.len(),
        "failed to move reservation queuers to reservations",
    )?;
    let affected = sqlx::query!(
        "delete from product_reservation_queuers
        where product_id = $1
        and user_id = any($2)",
        ticket_kind,
        &new_reservations
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        new_reservations.len() - pulled.len(),
        "failed to remove promoted reservation queuers",
    )?;
    Ok(new_reservations)
}

pub(super) async fn give_reservations_in_new_transaction(
    ctx: &ContextWrapper,
    ticket_kind: Uuid,
    fetch_n: i32,
) -> MinilithResult<()> {
    let mut txn = ctx.db.begin().await?;
    let promoted = give_reservations(ticket_kind, fetch_n, &mut txn).await?;
    txn.commit().await?;
    notify_ticket_users(ctx, ticket_kind, promoted, TicketNotification::Reservation);
    Ok(())
}
/// Clear reservation queues that can never be served again: their kind is sold out and nobody
/// is left holding a reservation for it, so no unit can be freed by a cancelled or timed out
/// payment. Queuers that were pushed down from a non-base kind keep their place while that kind
/// can still free a unit.
pub(super) async fn remove_queuers_when_sold_out(db: &mut Transaction<'_>) -> MinilithResult<()> {
    let queuers = sqlx::query_scalar!(
        r#"select flow.user_id
        from users_in_purchase_flow flow
        inner join product_reservation_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.product_id = flow.product_id
        inner join products queue_kind on queue_kind.id = queuer.product_id
        inner join products wanted
            on wanted.id = coalesce(queuer.original_product_id, queuer.product_id)
        where flow.reservation_queue = flow.user_id
        -- neither the kind they are queued in nor the kind they want can free a unit anymore
        and coalesce(queue_kind.sales_stock, 0) <= 0
        and not exists (
            select 1 from product_reservations reservation
            where reservation.product_id = queue_kind.id
        )
        and coalesce(wanted.sales_stock, 0) <= 0
        and not exists (
            select 1 from product_reservations reservation
            where reservation.product_id = wanted.id
        )
        order by flow.user_id
        for update of flow skip locked"#
    )
    .fetch_all(&mut db.executor())
    .await?;
    if queuers.is_empty() {
        return Ok(());
    }
    let affected = sqlx::query!(
        "delete from product_reservation_queuers where user_id = any($1)",
        &queuers
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        queuers.len(),
        "failed to clear sold-out reservation queue",
    )?;
    unlist_users_purchase_flow(db, &queuers).await
}
