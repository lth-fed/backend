use std::sync::Arc;
use std::{collections::HashMap, ops::ControlFlow};

use bin_common::{PgPool, Transaction};
use rand::{rng, seq::SliceRandom as _};
use sqlx::postgres::types::PgInterval;
use uuid::Uuid;

use super::{
    allocation::{
        give_reservations, new_timeout_interval, remove_queuers_when_sold_out, remove_reservation,
        reserve_ticket_capacity,
    },
    ensure_affected_rows,
    flow::{PurchaseFlow, unlist_users_purchase_flow, wait_for_user_purchase_flow},
};
use crate::{
    ContextWrapper, InternationalizedString as IS, MinilithEndpointError, MinilithResult,
    push_notifications::{NotificationRow, PushDeviceRow, send_notifications},
};

async fn release_next_ticket(ctx: &ContextWrapper) -> MinilithResult<ControlFlow<()>> {
    // Commit the claim before acquiring flow locks. If the process dies
    // after this, the misplaced-queuer pass completes the release.
    let ticket_kind = sqlx::query_scalar!(
        "with next as (
            select id from ticket_kinds
            where purchasing_available_start > now() - '5 minutes'::interval
            and purchasing_available_start <= now() + '40 seconds'::interval
            and has_been_released = false
            order by id
            limit 1
            for update skip locked
        )
        update ticket_kinds kind
        set has_been_released = true
        from next
        where kind.id = next.id
        returning kind.id"
    )
    .fetch_optional(&ctx.db)
    .await?;
    if let Some(ticket_kind) = ticket_kind {
        let release_txn = ctx.db.begin().await?;
        release(Some(ctx), release_txn, ticket_kind).await?;
        Ok(ControlFlow::Continue(()))
    } else {
        // there may in certain circumstances be "jobs" left, but they will be taken care of
        // next minute in worst case
        Ok(ControlFlow::Break(()))
    }
}
async fn send_release_notifications(
    ctx: &ContextWrapper,
    id: Uuid,
    activity_id: Uuid,
    reservation_devices: Vec<PushDeviceRow>,
    reservation_queue_devices: Vec<PushDeviceRow>,
) -> MinilithResult<()> {
    let notification = NotificationRow {
        id,
        activity_id: Some(activity_id),
        // people know where it's from since they just used the app
        sender: sqlx::types::Json(IS::empty()),
        title: IS(HashMap::from_iter([
            ("sv".to_owned(), "Gå in och köp biljetten!".to_owned()),
            (
                "en".to_owned(),
                "Open the app to buy your ticket!".to_owned(),
            ),
        ]))
        .into(),
        content: IS(HashMap::from_iter([(
            "sv".to_owned(),
            "Du fick en reservation. Köp biljetten snart, annars får någon annan din reservation."
                .to_owned(),
        ),
            (
                "en".to_owned(),
                "You got a reservation. Buy the ticket soon, else someone else will get your reservation.".to_owned(),
            ),
        ]))
        .into(),
    };
    // errors are logged & alerted when creating MinilithEndpointError
    let removed1 = send_notifications(ctx, &notification, reservation_devices).await;
    let notification = NotificationRow {
        id,
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
        content: IS(HashMap::from_iter([
            (
                "sv".to_owned(),
                "Du har en plats i kön till reservationer, \
                så om någon avbryter sitt köp får du reservationen."
                    .to_owned(),
            ),
            (
                "en".to_owned(),
                "You have a spot in the queue to reservations. \
                If someone cancels their payment, you get a reservation."
                    .to_owned(),
            ),
        ]))
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
/// Releases a ticket. This MUST be called at the moment the tickets should be released.
/// It MUST have locked the `ticket_kind`.
///
/// # Errors
///
/// Db errors or if the id doesn't exist.
#[allow(
    clippy::too_many_lines,
    reason = "keeps the bulk flow lock and both state transitions in execution order"
)]
pub(super) async fn release(
    ctx: Option<&ContextWrapper>,
    mut db: Transaction<'_>,
    id: Uuid,
) -> MinilithResult<()> {
    let ticket_kind = sqlx::query!(
        "select *
        from ticket_kinds 
        where id = $1",
        id
    )
    .fetch_one(&mut db.executor())
    .await?;

    // Purchase-flow rows are the first mutable rows locked by every path.
    // Sorting makes bulk acquisition deterministic.
    let mut queuers = sqlx::query_scalar!(
        "select flow.user_id
        from users_in_purchase_flow flow
        inner join ticket_release_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.ticket_kind_id = flow.ticket_kind_id
        where flow.ticket_kind_id = $1
        and flow.release_queue = flow.user_id
        order by flow.user_id
        for update of flow skip locked",
        id
    )
    .fetch_all(&mut db.executor())
    .await?;

    queuers.shuffle(&mut rng());

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we won't have that many queuers"
    )]
    let requested = (ticket_kind.max_tickets - ticket_kind.reserved_or_purchased_tickets)
        .min(queuers.len() as i32)
        .max(0);
    let granted = reserve_ticket_capacity(&mut db, id, requested).await?;
    #[allow(
        clippy::cast_sign_loss,
        reason = "i goddamn hope not granted will be negative"
    )]
    let (reservations, reservation_queuers) = queuers.split_at(granted as usize);

    let timestamps: Vec<PgInterval> = std::iter::repeat_with(new_timeout_interval)
        .take(reservations.len())
        .collect();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we know it won't, we hopefully don't have more than 2G reservations..."
    )]
    let placements: Vec<i32> = reservation_queuers
        .iter()
        .enumerate()
        .map(|(idx, _)| (idx + reservations.len() + 1) as i32)
        .collect();

    // we don't check the amount of affected here in case some dropped out or similar
    sqlx::query!(
        "insert into ticket_reservations (user_id, ticket_kind_id, transaction_id, timeout)
        select user_id, $2 as ticket_kind_id, null as transaction_id, (from_now + now()) as timeout
        from unnest($1::text[], $3::interval[]) as t(user_id, from_now)",
        reservations,
        id,
        &timestamps
    )
    .execute(&mut db.executor())
    .await?;
    sqlx::query!(
        "insert into ticket_reservation_queuers (user_id, ticket_kind_id, placement)
        select user_id, $2 as ticket_kind_id, placement
        from unnest($1::text[], $3::integer[]) as t(user_id, placement)",
        reservation_queuers,
        id,
        &placements
    )
    .execute(&mut db.executor())
    .await?;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we know it won't, we hopefully don't have more than 2G reservations..."
    )]
    sqlx::query!(
        "insert into ticket_reservation_placement_tails (ticket_kind_id, placement_tail)
        values ($1, $2)",
        id,
        (reservations.len() + reservation_queuers.len() + 1) as i32,
    )
    .execute(&mut db.executor())
    .await?;
    sqlx::query!(
        "update users_in_purchase_flow
            set release_queue = null, reservation = user_id
        where user_id = any($1)
        and ticket_kind_id = $2
        and release_queue = user_id
        and reservation_queue is null
        and reservation is null",
        &reservations,
        id,
    )
    .execute(&mut db.executor())
    .await?;
    sqlx::query!(
        "update users_in_purchase_flow
            set release_queue = null, reservation_queue = user_id
        where user_id = any($1)
        and ticket_kind_id = $2
        and release_queue = user_id
        and reservation_queue is null
        and reservation is null",
        &reservation_queuers,
        id,
    )
    .execute(&mut db.executor())
    .await?;

    sqlx::query!(
        "delete from ticket_release_queuers
        where user_id = any ($1)",
        &queuers
    )
    .execute(&mut db.executor())
    .await?;

    let reservation_devices = sqlx::query_as!(
        PushDeviceRow,
        "select user_id, device_id, push_token, language,
        platform as \"platform!: crate::push_notifications::PushPlatform\"
        from users
        join push_devices on push_devices.user_id = users.id
        where users.id = any($1)",
        &reservations
    )
    .fetch_all(&mut db.executor())
    .await?;
    let reservation_queue_devices = sqlx::query_as!(
        PushDeviceRow,
        "select user_id, device_id, push_token, language,
        platform as \"platform!: crate::push_notifications::PushPlatform\"
        from users
        join push_devices on push_devices.user_id = users.id
        where users.id = any($1)",
        &reservation_queuers
    )
    .fetch_all(&mut db.executor())
    .await?;
    db.commit().await?;
    let Some(ctx) = ctx else {
        return Ok(());
    };
    let ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        // MinilithEndpointError creation causes error! & alert
        drop(
            send_release_notifications(
                &ctx,
                id,
                ticket_kind.activity_id,
                reservation_devices,
                reservation_queue_devices,
            )
            .await,
        );
    });
    Ok(())
}

async fn remove_reservation_tails(db: &PgPool) -> MinilithResult<()> {
    sqlx::query_scalar!(
        "delete from ticket_reservation_placement_tails tails
        using ticket_kinds kind
        where kind.id = tails.ticket_kind_id
            and purchasing_available_stop < now()"
    )
    .execute(db)
    .await?;
    Ok(())
}
pub(super) async fn remove_expired_release_queuers(db: &mut Transaction<'_>) -> MinilithResult<()> {
    let user_ids = sqlx::query_scalar!(
        "select flow.user_id
        from users_in_purchase_flow flow
        inner join ticket_release_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.ticket_kind_id = flow.ticket_kind_id
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
        "delete from ticket_release_queuers where user_id = any($1)",
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
            "select flow.user_id, flow.ticket_kind_id
            from users_in_purchase_flow flow
            inner join ticket_release_queuers queuer
                on queuer.user_id = flow.user_id
                and queuer.ticket_kind_id = flow.ticket_kind_id
            inner join ticket_kinds kind on kind.id = flow.ticket_kind_id
            where kind.has_been_released = true
            and flow.release_queue = flow.user_id
            order by flow.user_id
            limit 1
            for update of flow skip locked"
        )
        .fetch_optional(&mut txn.executor())
        .await?;
        if let Some(row) = ticket_kind {
            update_misplaced_queuer(&row.user_id, row.ticket_kind_id, &mut txn).await?;
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
    let mut reservations = sqlx::query!(
        "select id as \"ticket_kind_id!\", -- count(user_id),
        (max_tickets - reserved_or_purchased_tickets) as \"available_tickets!\"
        from ticket_reservation_queuers
        inner join ticket_kinds kind on (kind.id = ticket_kind_id)
        where max_tickets > reserved_or_purchased_tickets
        group by id"
    )
    .fetch_all(&ctx.db)
    .await?;
    // shuffle because if multiple runners are trying to do this, make each start at a different
    // node so we don't get as many "for update skip locked" in the start:)
    reservations.shuffle(&mut rng());
    for reservation in reservations {
        let mut txn = ctx.db.begin().await?;
        give_reservations(
            reservation.ticket_kind_id,
            reservation.available_tickets,
            &mut txn,
        )
        .await?;
        txn.commit().await?;
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
    sqlx::query_scalar!(
        "select id from ticket_kinds where id = $1 for update",
        ticket_kind
    )
    .fetch_one(&mut db.executor())
    .await?;

    let affected = sqlx::query!(
        "with placement as (
            update ticket_reservation_placement_tails
                set placement_tail = placement_tail + 1
            where ticket_kind_id = $2
            returning old.placement_tail
        )
        insert into ticket_reservation_queuers (user_id, ticket_kind_id, placement)
        select $1 as user_id, $2 as ticket_kind_id, placement.placement_tail as placement
        from placement",
        user_id,
        ticket_kind
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to insert misplaced user into reservation queue",
    )?;
    let affected = sqlx::query!(
        "update users_in_purchase_flow
        set release_queue = null, reservation_queue = user_id
        where user_id = $1
        and ticket_kind_id = $2
        and release_queue = user_id
        and reservation_queue is null
        and reservation is null",
        user_id,
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
        "delete from ticket_release_queuers where user_id = $1",
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to remove misplaced release queuer",
    )
}
