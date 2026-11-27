use std::sync::Arc;
use std::{collections::HashMap, ops::ControlFlow};

use bin_common::{PgPool, Transaction};
use rand::{rng, seq::SliceRandom as _};
use sqlx::postgres::types::PgInterval;
use uuid::Uuid;

use super::{
    allocation::{
        give_reservations_in_new_transaction, guaranteed_invitees, new_timeout_interval,
        public_capacity, queue_base, remove_queuers_when_sold_out, remove_reservation,
        reserve_ticket_capacity,
    },
    ensure_affected_rows,
    flow::{PurchaseFlow, unlist_users_purchase_flow, wait_for_user_purchase_flow},
    notifications::{reservation_notification, with_product_name},
};
use crate::{
    ContextWrapper, InternationalizedString as IS, MinilithEndpointError, MinilithResult,
    push_notifications::{NotificationKind, NotificationRow, PushDeviceRow, send_notifications},
};

async fn release_next_ticket(ctx: &ContextWrapper) -> MinilithResult<ControlFlow<()>> {
    // Commit the claim before acquiring flow locks. If the process dies
    // after this, the misplaced-queuer pass completes the release.
    let claimed = sqlx::query!(
        r#"with due as (
            select kind.id, kind.sales_start,
                coalesce((
                    select activity.base_ticket
                    from product_activity_assignments assignment
                    inner join activities activity on activity.id = assignment.activity_id
                    where assignment.product_id = kind.id
                    order by activity.id
                    limit 1
                ), kind.id) as base_ticket
            from products kind
            where kind.sales_start > now() - '5 minutes'::interval
            and kind.sales_start <= now() + '40 seconds'::interval
            and kind.tickets_been_released = false
            and kind.kind = 'ticket'
        ), chosen as (
            -- One group at a time: every kind that becomes purchasable at the same moment
            -- (`sales_start`) of the same activity is drawn together.
            select due.base_ticket, due.sales_start
            from due
            group by due.base_ticket, due.sales_start
            order by due.sales_start, due.base_ticket
            limit 1
        ), guard as (
            -- The base ticket row is the group's mutex. Only the replica holding it claims the
            -- group, so a group is never drawn in halves by two replicas.
            select kind.id
            from products kind
            inner join chosen on chosen.base_ticket = kind.id
            for update of kind skip locked
        ), claimed as (
            update products kind
            set tickets_been_released = true
            from due
            inner join chosen
                on chosen.base_ticket = due.base_ticket
                and chosen.sales_start = due.sales_start
            where kind.id = due.id
            and exists (select 1 from guard)
            returning kind.id
        )
        select claimed.id, due.sales_start, due.base_ticket as "base_ticket!"
        from claimed
        inner join due on due.id = claimed.id"#
    )
    .fetch_all(&ctx.db)
    .await?;
    let Some(first) = claimed.first() else {
        // there may in certain circumstances be "jobs" left, but they will be taken care of
        // next minute in worst case
        return Ok(ControlFlow::Break(()));
    };
    let base_ticket = first.base_ticket;
    let kinds: Vec<Uuid> = claimed.iter().map(|row| row.id).collect();
    if kinds.contains(&base_ticket) {
        let release_txn = ctx.db.begin().await?;
        release_group(Some(ctx), release_txn, base_ticket, kinds).await?;
    } else {
        // The base ticket is not released right now (already released, or released at another
        // moment), so these kinds are drawn on their own and leftovers stay in their own queue.
        for kind in kinds {
            let release_txn = ctx.db.begin().await?;
            release_group(Some(ctx), release_txn, kind, vec![kind]).await?;
        }
    }
    Ok(ControlFlow::Continue(()))
}
async fn send_release_notifications(
    ctx: &ContextWrapper,
    id: Uuid,
    activity_id: Uuid,
    product_name: &IS,
    reservation_devices: Vec<PushDeviceRow>,
    reservation_queue_devices: Vec<PushDeviceRow>,
) -> MinilithResult<()> {
    let notification = reservation_notification(id, activity_id, product_name);
    // errors are logged & alerted when creating MinilithEndpointError
    let removed1 = send_notifications(ctx, &notification, reservation_devices).await;
    let notification = NotificationRow {
        id,
        kind: NotificationKind::Reservation,
        activity_id: Some(activity_id),
        // people know where it's from since they just used the app
        sender: sqlx::types::Json(IS::empty()),
        title: IS(HashMap::from_iter([
            ("sv".to_owned(), "Se din plats i kön".to_owned()),
            (
                "en".to_owned(),
                "Tap to view your reservation placement".to_owned(),
            ),
        ]))
        .into(),
        content: with_product_name(
            product_name,
            "Du har en plats i kön till {}, så om någon avbryter sitt köp får du reservationen.",
            "You have a spot in the queue for {}. \
            If someone cancels their payment, you get a reservation.",
        )
        .into(),
    };
    let removed2 = send_notifications(ctx, &notification, reservation_queue_devices).await;

    let mut txn = ctx.db.begin().await?;
    if let Ok(removed) = removed1 {
        removed.clear_failed(&mut txn).await?;
    }
    if let Ok(removed) = removed2 {
        removed.clear_failed(&mut txn).await?;
    }
    txn.commit().await?;
    Ok(())
}
/// Locks and returns the users waiting in the release queue of `kind`.
async fn release_queuers(db: &mut Transaction<'_>, kind: Uuid) -> MinilithResult<Vec<String>> {
    // Purchase-flow rows are the first mutable rows locked by every path.
    // Sorting makes bulk acquisition deterministic.
    let queuers = sqlx::query_scalar!(
        "select flow.user_id
        from users_in_purchase_flow flow
        inner join product_release_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.product_id = flow.product_id
        where flow.product_id = $1
        and flow.release_queue = flow.user_id
        order by flow.user_id
        for update of flow skip locked",
        kind
    )
    .fetch_all(&mut db.executor())
    .await?;
    Ok(queuers)
}

/// Shuffles `queuers` and reserves up to the product's remaining stock for the first ones.
/// Invitees go first: [`public_capacity`] holds their promised units back from everybody else.
///
/// Returns the users that got a reservation and the ones that did not.
async fn draw(
    db: &mut Transaction<'_>,
    kind: Uuid,
    mut queuers: Vec<String>,
) -> MinilithResult<(Vec<String>, Vec<String>)> {
    queuers.shuffle(&mut rng());
    let invitees = guaranteed_invitees(&mut db.executor(), kind).await?;
    queuers.sort_by_key(|user| !invitees.contains(user));
    let invited_queued = queuers
        .iter()
        .filter(|user| invitees.contains(*user))
        .count();
    let public = public_capacity(db, kind).await?;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we won't have that many queuers"
    )]
    let requested = (public + invited_queued as i32)
        .min(queuers.len() as i32)
        .max(0);
    let granted = reserve_ticket_capacity(db, kind, requested, false).await?;
    #[allow(
        clippy::cast_sign_loss,
        reason = "granted is capped by the number of queuers"
    )]
    let leftovers = queuers.split_off(granted as usize);
    Ok((queuers, leftovers))
}

async fn insert_reservations(
    db: &mut Transaction<'_>,
    kind: Uuid,
    reserved: &[String],
) -> MinilithResult<()> {
    if reserved.is_empty() {
        return Ok(());
    }
    let timestamps: Vec<PgInterval> = std::iter::repeat_with(new_timeout_interval)
        .take(reserved.len())
        .collect();
    let affected = sqlx::query!(
        "insert into product_reservations
            (user_id, product_id, product_price, transaction_id, timeout)
        select t.user_id, $2, base.price, null, (t.from_now + now())
        from unnest($1::text[], $3::interval[]) as t(user_id, from_now)
        inner join products kind on kind.id = $2
        inner join product_base_prices base on base.product_id = kind.id",
        reserved,
        kind,
        &timestamps
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        reserved.len(),
        "failed to create released ticket reservations",
    )?;
    sqlx::query!(
        "update users_in_purchase_flow
            set release_queue = null, reservation = user_id
        where user_id = any($1)
        and product_id = $2
        and release_queue = user_id
        and reservation_queue is null
        and reservation is null",
        reserved,
        kind,
    )
    .execute(&mut db.executor())
    .await?;
    let affected = sqlx::query!(
        "delete from product_release_queuers
        where user_id = any($1)",
        reserved
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        reserved.len(),
        "failed to remove reserved release queuers",
    )?;
    Ok(())
}

async fn push_devices(
    db: &mut Transaction<'_>,
    user_ids: &[String],
) -> MinilithResult<Vec<PushDeviceRow>> {
    let devices = sqlx::query_as!(
        PushDeviceRow,
        "select user_id, device_id, push_token, language,
        platform as \"platform!: crate::push_notifications::PushPlatform\"
        from users
        join push_devices on push_devices.user_id = users.id
        where users.id = any($1)",
        user_ids
    )
    .fetch_all(&mut db.executor())
    .await?;
    Ok(devices)
}

async fn deliver_release_notifications(
    ctx: ContextWrapper,
    kind: Uuid,
    reservation_devices: Vec<PushDeviceRow>,
    reservation_queue_devices: Vec<PushDeviceRow>,
) -> MinilithResult<()> {
    let product = sqlx::query!(
        r#"select assignment.activity_id as "activity_id!",
            kind.name as "name!: crate::DbInternationalizedString"
        from products kind
        inner join product_activity_assignments assignment on assignment.product_id = kind.id
        where kind.id = $1
        limit 1"#,
        kind
    )
    .fetch_optional(&ctx.db)
    .await?
    .ok_or_else(|| {
        MinilithEndpointError::bad_frontend_code(
            "ticket release without an activity",
            "product_activity_assignments",
        )
    })?;
    send_release_notifications(
        &ctx,
        kind,
        product.activity_id,
        &product.name.0,
        reservation_devices,
        reservation_queue_devices,
    )
    .await
}

/// Puts `rows` (user, original kind, placement) in the reservation queue of `kind`. The original
/// kind is written per group of users so no nullable array parameter is needed.
async fn enqueue(
    db: &mut Transaction<'_>,
    kind: Uuid,
    rows: &[(String, Option<Uuid>, i32)],
) -> MinilithResult<()> {
    let mut by_origin: HashMap<Option<Uuid>, (Vec<String>, Vec<i32>)> = HashMap::new();
    for (user, origin, placement) in rows {
        let bucket = by_origin.entry(*origin).or_default();
        bucket.0.push(user.clone());
        bucket.1.push(*placement);
    }
    for (origin, (users, placements)) in by_origin {
        let affected = sqlx::query!(
            "insert into product_reservation_queuers
                (user_id, product_id, original_product_id, placement)
            select user_id, $2 as product_id, $3 as original_product_id, placement
            from unnest($1::text[], $4::integer[]) as t(user_id, placement)",
            &users,
            kind,
            origin,
            &placements
        )
        .execute(&mut db.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            users.len(),
            "failed to insert the base ticket reservation queue",
        )?;
    }
    Ok(())
}

/// Draws a whole release group. Every kind is drawn from its own stock in a random order, the
/// non-base kinds first. Everybody a non-base kind can't fit falls back into the base ticket's
/// lottery, which is drawn last. Whoever the base ticket can't fit either stays in the base
/// ticket's reservation queue.
///
/// The caller must have claimed the kinds (`tickets_been_released = true`).
#[allow(
    clippy::too_many_lines,
    reason = "keeps the bulk flow lock and all state transitions in execution order"
)]
pub(super) async fn release_group(
    ctx: Option<&ContextWrapper>,
    mut db: Transaction<'_>,
    base_ticket: Uuid,
    kinds: Vec<Uuid>,
) -> MinilithResult<()> {
    let extra_kinds: Vec<Uuid> = kinds
        .iter()
        .copied()
        .filter(|kind| *kind != base_ticket)
        .collect();
    let mut extras: Vec<(Uuid, Vec<String>)> = Vec::new();
    // user -> the non-base kind they originally queued for
    let mut fallback: HashMap<String, Uuid> = HashMap::new();
    for kind in &extra_kinds {
        let queuers = release_queuers(&mut db, *kind).await?;
        let (reserved, leftovers) = draw(&mut db, *kind, queuers).await?;
        fallback.extend(leftovers.into_iter().map(|user| (user, *kind)));
        extras.push((*kind, reserved));
    }

    if !fallback.is_empty() {
        let fallen: Vec<String> = fallback.keys().cloned().collect();
        // The flow has to point at the base ticket before any of its child rows are written,
        // because the flow → queue foreign keys are immediate.
        let affected = sqlx::query!(
            "delete from product_release_queuers
            where user_id = any($1)",
            &fallen
        )
        .execute(&mut db.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            fallen.len(),
            "failed to remove the release queues of the fallen-through users",
        )?;
        let affected = sqlx::query!(
            "update users_in_purchase_flow
            set product_id = $2
            where user_id = any($1) and product_id = any($3) and release_queue = user_id",
            &fallen,
            base_ticket,
            &extra_kinds
        )
        .execute(&mut db.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            fallen.len(),
            "failed to push ticket-kind leftovers down to the base ticket",
        )?;
    }

    let mut base_queuers = release_queuers(&mut db, base_ticket).await?;
    base_queuers.extend(fallback.keys().cloned());
    let (base_reservations, base_queue) = draw(&mut db, base_ticket, base_queuers).await?;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we won't have that many queuers"
    )]
    let placements: Vec<i32> = base_queue
        .iter()
        .enumerate()
        .map(|(idx, _)| (idx + base_reservations.len() + 1) as i32)
        .collect();
    // Remember the non-base kind each queuer originally wanted, so a unit of that kind that
    // frees up later can be offered back to them (see `give_reservations`).
    let queued: Vec<(String, Option<Uuid>, i32)> = base_queue
        .into_iter()
        .zip(placements)
        .map(|(user, placement)| {
            let origin = fallback.get(&user).copied();
            (user, origin, placement)
        })
        .collect();
    let base_queue: Vec<String> = queued.iter().map(|(user, _, _)| user.clone()).collect();

    for (kind, reserved) in &extras {
        insert_reservations(&mut db, *kind, reserved).await?;
    }
    insert_reservations(&mut db, base_ticket, &base_reservations).await?;

    enqueue(&mut db, base_ticket, &queued).await?;
    if !base_queue.is_empty() {
        sqlx::query!(
            "update users_in_purchase_flow
                set release_queue = null, reservation_queue = user_id
            where user_id = any($1)
            and product_id = $2
            and release_queue = user_id
            and reservation_queue is null
            and reservation is null",
            &base_queue,
            base_ticket,
        )
        .execute(&mut db.executor())
        .await?;
        let affected = sqlx::query!(
            "delete from product_release_queuers
            where user_id = any($1)",
            &base_queue
        )
        .execute(&mut db.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            base_queue.len(),
            "failed to remove the queued release queuers",
        )?;
    }

    // Only the base ticket keeps a reservation queue, so only it needs a placement tail.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we won't have that many queuers"
    )]
    let tail = (base_reservations.len() + base_queue.len() + 1) as i32;
    sqlx::query!(
        "insert into product_reservation_placement_tails (product_id, placement_tail)
        values ($1, $2)",
        base_ticket,
        tail,
    )
    .execute(&mut db.executor())
    .await?;

    let mut tails = extras;
    tails.push((base_ticket, base_reservations));
    let mut notifications = Vec::new();
    for (kind, reserved) in &tails {
        let reserved_devices = push_devices(&mut db, reserved).await?;
        let queued_devices = if *kind == base_ticket {
            push_devices(&mut db, &base_queue).await?
        } else {
            Vec::new()
        };
        if !reserved_devices.is_empty() || !queued_devices.is_empty() {
            notifications.push((*kind, reserved_devices, queued_devices));
        }
    }

    db.commit().await?;
    let Some(ctx) = ctx else {
        return Ok(());
    };
    for (kind, reservation_devices, reservation_queue_devices) in notifications {
        let ctx = Arc::clone(ctx);
        tokio::spawn(async move {
            // MinilithEndpointError creation causes error! & alert
            drop(
                deliver_release_notifications(
                    ctx,
                    kind,
                    reservation_devices,
                    reservation_queue_devices,
                )
                .await,
            );
        });
    }
    Ok(())
}

async fn remove_reservation_tails(db: &PgPool) -> MinilithResult<()> {
    sqlx::query_scalar!(
        "delete from product_reservation_placement_tails tails
        using products kind
        where kind.id = tails.product_id
            and kind.sales_stop < now()"
    )
    .execute(db)
    .await?;
    Ok(())
}
pub(super) async fn remove_expired_release_queuers(db: &mut Transaction<'_>) -> MinilithResult<()> {
    let user_ids = sqlx::query_scalar!(
        "select flow.user_id
        from users_in_purchase_flow flow
        inner join product_release_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.product_id = flow.product_id
        where queuer.started_queueing < now() - '20 minutes'::interval
        and flow.release_queue = flow.user_id
        order by flow.user_id
        for update of flow skip locked"
    )
    .fetch_all(&mut db.executor())
    .await?;
    if user_ids.is_empty() {
        return Ok(());
    }

    let affected = sqlx::query!(
        "delete from product_release_queuers where user_id = any($1)",
        &user_ids
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        user_ids.len(),
        "failed to remove expired release queuers",
    )?;
    unlist_users_purchase_flow(db, &user_ids).await
}

/// Run this xx:01 (i.e. seconds = 01 & every minute).
/// All DB things this does have to be compatible with being called simultaneously because we might
/// horizontally scale.
///
/// # Errors
///
/// Db. See [`release`].
pub(crate) async fn check_all_tickets(ctx: &ContextWrapper) -> MinilithResult<()> {
    // release tickets
    loop {
        if release_next_ticket(ctx).await?.is_break() {
            break;
        }
    }
    remove_reservation_tails(&ctx.db).await?;

    // Also removes people's queue positions after 20 minutes.
    let mut txn = ctx.db.begin().await?;
    remove_expired_release_queuers(&mut txn).await?;
    txn.commit().await?;

    // missplaced
    loop {
        let mut txn = ctx.db.begin().await?;
        // take one release job
        // this works concurrently too!
        let ticket_kind = sqlx::query!(
            "select flow.user_id, flow.product_id
            from users_in_purchase_flow flow
            inner join product_release_queuers queuer
                on queuer.user_id = flow.user_id
                and queuer.product_id = flow.product_id
            inner join products kind on kind.id = flow.product_id
            where kind.tickets_been_released = true
            and flow.release_queue = flow.user_id
            order by flow.user_id
            limit 1
            for update of flow skip locked"
        )
        .fetch_optional(&mut txn.executor())
        .await?;
        if let Some(row) = ticket_kind {
            update_misplaced_queuer(&row.user_id, row.product_id, &mut txn).await?;
            txn.commit().await?;
        } else {
            // there may in certain circumstances be "jobs" left, but they will be taken care of
            // next minute in worst case
            break;
        }
    }

    // remove_reservation
    while remove_reservation(&ctx.db).await?.is_continue() {}

    // give_reservations:
    // Also visit the kinds that pushed-down queuers are waiting for, so a unit that frees up
    // (a cancelled or timed out reservation) is offered back to them.
    let mut reservations = sqlx::query!(
        r#"with wanted as (
            select product_id as id from product_reservation_queuers
            union
            select coalesce(original_product_id, product_id) as id
            from product_reservation_queuers
        )
        select kind.id as "product_id!",
        coalesce(kind.sales_stock, 0) as "available_tickets!"
        from products kind
        inner join wanted on wanted.id = kind.id
        where coalesce(kind.sales_stock, 0) > 0"#
    )
    .fetch_all(&ctx.db)
    .await?;
    // shuffle because if multiple runners are trying to do this, make each start at a different
    // node so we don't get as many "for update skip locked" in the start:)
    reservations.shuffle(&mut rng());
    for reservation in reservations {
        give_reservations_in_new_transaction(
            ctx,
            reservation.product_id,
            reservation.available_tickets,
        )
        .await?;
    }

    let mut txn = ctx.db.begin().await?;
    remove_queuers_when_sold_out(&mut txn).await?;
    txn.commit().await?;

    Ok(())
}
/// Checks for people in the queue once the `ticket_kind` has been released.
/// Assumed that the `ticket_kind` is released.
///
/// # Errors
///
/// DB.
pub(super) async fn update_misplaced_queuer(
    user_id: &str,
    ticket_kind: Uuid,
    db: &mut Transaction<'_>,
) -> MinilithResult<()> {
    let flow = wait_for_user_purchase_flow(db, user_id, Some(ticket_kind), None)
        .await?
        .ok_or_else(|| {
            MinilithEndpointError::internal_error(
                "misplaced release queuer has no purchase flow",
                user_id,
            )
        })?;
    if *flow != PurchaseFlow::ReleaseQueue {
        return Err(MinilithEndpointError::internal_error(
            "misplaced release queuer has wrong purchase-flow state",
            flow,
        ));
    }
    // Only the base ticket has a reservation queue: the user queues there and keeps the kind
    // they wanted.
    let queue_kind = queue_base(&mut db.executor(), ticket_kind).await?;
    sqlx::query_scalar!(
        "select id from products where id = $1 for update",
        queue_kind
    )
    .fetch_one(&mut db.executor())
    .await?;
    let wanted = (queue_kind != ticket_kind).then_some(ticket_kind);

    let affected = sqlx::query!(
        "delete from product_release_queuers where user_id = $1",
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to remove misplaced release queuer",
    )?;
    let affected = sqlx::query!(
        "update users_in_purchase_flow
        set product_id = $2, release_queue = null, reservation_queue = user_id
        where user_id = $1
        and product_id = $3
        and release_queue = user_id
        and reservation_queue is null
        and reservation is null",
        user_id,
        queue_kind,
        ticket_kind,
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to move misplaced user's purchase flow",
    )?;
    let affected = sqlx::query!(
        "with placement as (
            update product_reservation_placement_tails
                set placement_tail = placement_tail + 1
            where product_id = $2
            returning old.placement_tail
        )
        insert into product_reservation_queuers
            (user_id, product_id, original_product_id, placement)
        select $1 as user_id, $2 as product_id, $3 as original_product_id,
            placement.placement_tail as placement
        from placement",
        user_id,
        queue_kind,
        wanted
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to insert misplaced user into reservation queue",
    )
}
