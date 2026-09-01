use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use sqlx::types::time::OffsetDateTime;

use super::{
    access::ensure_user_may_purchase_ticket,
    allocation::{give_reservations, new_timeout_interval, reserve_ticket_capacity},
    ensure_affected_rows,
    flow::{
        PurchaseFlow, lock_user_purchase_flow, reserve_user_purchase_flow,
        set_user_purchase_flow_release_queue, set_user_purchase_flow_reservation,
        set_user_purchase_flow_reservation_queue, unlist_user_purchase_flow,
        wait_for_user_purchase_flow,
    },
    models::{PurchaseStatus, QueueRequest, QueueResponse},
};
use crate::{ContextWrapper, MinilithEndpointError, MinilithResult};

#[allow(
    clippy::too_many_lines,
    reason = "keeps the three purchase-flow transitions and their lock order together"
)]
pub(super) async fn queue(
    ctx: &ContextWrapper,
    user: User,
    req: QueueRequest,
) -> MinilithResult<PurchaseStatus> {
    let mut txn = ctx.db.begin().await?;
    ensure_user_may_purchase_ticket(&mut txn.executor(), user.get_id(), req.ticket_kind).await?;
    // TODO(frontend-hack: 25/08/2026): frontend doesn't send a DELETE to /queue so we delete it here in case the user
    // wants to queue for a different ticket kind.
    match lock_user_purchase_flow(&mut txn, user.get_id(), None, None).await {
        Ok(flow) if flow.ticket_kind_id() != req.ticket_kind => {
            let (mut txn, ticket_kind_to_fill) = flow.cancel(ctx, user.get_id(), txn).await?;
            unlist_user_purchase_flow(&mut txn, user.get_id()).await?;

            if let Some(ticket_kind) = ticket_kind_to_fill {
                drop(give_reservations(ticket_kind, 1, &mut txn).await);
            }

            txn.commit().await?;
            return Err(MinilithEndpointError::bad_frontend_code(
                "your current flow is cancelled, press the button again to place in this queue",
                "",
            ));
        }
        // continue as before
        Ok(_) | Err(MinilithEndpointError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    reserve_user_purchase_flow(
        &mut txn,
        &[user.get_id().to_owned()],
        req.ticket_kind,
        &[false],
    )
    .await?;
    let row = sqlx::query!(
        "select has_been_released, purchasing_available_stop from ticket_kinds where id = $1",
        req.ticket_kind
    )
    .fetch_one(&mut txn.executor())
    .await?;

    if row.purchasing_available_stop < OffsetDateTime::now_utc() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "ticket not available for purchase anymore",
            "",
        ));
    }

    if row.has_been_released {
        let row = sqlx::query!(
            "select (
                    select count(user_id) from ticket_reservation_queuers
                    where ticket_kind_id = $1
                ) as \"count!\",
                reserved_or_purchased_tickets, max_tickets
                from ticket_kinds where id = $1",
            req.ticket_kind
        )
        .fetch_one(&mut txn.executor())
        .await?;

        let reserved = reserve_ticket_capacity(&mut txn, req.ticket_kind, 1).await?;
        if reserved == 0 {
            return Err(MinilithEndpointError::bad_user_input(
                "the tickets are sold out",
                "",
                "the tickets are sold out",
                "ticket_kind_id",
            ));
        }
        if row.reserved_or_purchased_tickets < row.max_tickets && row.count == 0 && reserved == 1 {
            // give reservation
            let affected = sqlx::query!(
                "insert into ticket_reservations
                        (user_id, ticket_kind_id, transaction_id, timeout)
                        values ($1, $2, null, now() + $3)",
                user.get_id(),
                req.ticket_kind,
                new_timeout_interval()
            )
            .execute(&mut txn.executor())
            .await?;
            ensure_affected_rows(
                affected.rows_affected(),
                1,
                "failed to create immediate ticket reservation",
            )?;
            set_user_purchase_flow_reservation(&mut txn, user.get_id()).await?;

            txn.commit().await?;
            return Ok(PurchaseStatus::Reserved);
        }

        let affected = sqlx::query!(
            "with placement as (
                    update ticket_reservation_placement_tails
                        set placement_tail = placement_tail + 1
                    where ticket_kind_id = $2
                    returning old.placement_tail
                )
                insert into ticket_reservation_queuers (user_id, ticket_kind_id, placement) 
                select $1, $2, placement.placement_tail
                from placement

                on conflict (user_id) do update
                set ticket_kind_id = excluded.ticket_kind_id, placement = excluded.placement",
            user.get_id(),
            req.ticket_kind
        )
        .execute(&mut txn.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            1,
            "failed to enter reservation queue",
        )?;
        set_user_purchase_flow_reservation_queue(&mut txn, user.get_id()).await?;
        txn.commit().await?;
        Ok(PurchaseStatus::ReservationQueued)
    } else {
        let affected = sqlx::query!(
            "insert into ticket_release_queuers (user_id, ticket_kind_id, started_queueing) \
                select $1, $2, now()
                where not exists (
                    select 1 from purchased_tickets where ticket_kind_id = $2 and owner_id = $1
                )
                on conflict (user_id) do update
                set ticket_kind_id = excluded.ticket_kind_id,
                    started_queueing = excluded.started_queueing",
            user.get_id(),
            req.ticket_kind
        )
        .execute(&mut txn.executor())
        .await?;
        // if the ticket is released now we're either gonna be in the release or the
        // update_misplaced_queuer will handle us
        ensure_affected_rows(affected.rows_affected(), 1, "failed to enter release queue")?;
        set_user_purchase_flow_release_queue(&mut txn, user.get_id()).await?;
        txn.commit().await?;
        Ok(PurchaseStatus::ReleaseQueued)
    }
}
pub(super) async fn status(ctx: &ContextWrapper, user: User) -> MinilithResult<QueueResponse> {
    let mut txn = ctx.db.begin().await?;
    let flow = wait_for_user_purchase_flow(&mut txn, user.get_id(), None, None)
        .await?
        .wrap_err_not_found()?;
    match *flow {
        PurchaseFlow::Reservation => {
            let reservation = sqlx::query!(
                "select ticket_kind_id, timeout
                    from ticket_reservations
                    where user_id = $1",
                user.get_id()
            )
            .fetch_one(&mut txn.executor())
            .await?;
            Ok(QueueResponse {
                ticket_kind: reservation.ticket_kind_id,
                placement: Some(0),
                timeout: Some(reservation.timeout),
                start_transaction_before: Some(reservation.timeout - 1 * time::Duration::MINUTE),
            })
        }
        PurchaseFlow::ReservationQueue => {
            let reservation_queue = sqlx::query!(
                "select placement, reserved_or_purchased_tickets, ticket_kind_id
                    from ticket_reservation_queuers
                    inner join ticket_kinds on
                        (ticket_kinds.id = ticket_reservation_queuers.ticket_kind_id)
                    where user_id = $1",
                user.get_id()
            )
            .fetch_one(&mut txn.executor())
            .await?;
            Ok(QueueResponse {
                ticket_kind: reservation_queue.ticket_kind_id,
                placement: Some(
                    (reservation_queue.placement - reservation_queue.reserved_or_purchased_tickets)
                        .max(0),
                ),
                timeout: None,
                start_transaction_before: None,
            })
        }
        PurchaseFlow::ReleaseQueue => {
            let queuer = sqlx::query_scalar!(
                "select ticket_kind_id from ticket_release_queuers where user_id = $1",
                user.get_id()
            )
            .fetch_one(&mut txn.executor())
            .await?;
            Ok(QueueResponse {
                ticket_kind: queuer,
                placement: None,
                timeout: None,
                start_transaction_before: None,
            })
        }
    }
}
pub(super) async fn drop_transaction_flow(ctx: &ContextWrapper, user: User) -> MinilithResult<()> {
    let mut txn = ctx.db.begin().await?;
    let flow = lock_user_purchase_flow(&mut txn, user.get_id(), None, None).await?;
    let (mut txn, ticket_kind_to_fill) = flow.cancel(ctx, user.get_id(), txn).await?;
    unlist_user_purchase_flow(&mut txn, user.get_id()).await?;

    if let Some(ticket_kind) = ticket_kind_to_fill {
        drop(give_reservations(ticket_kind, 1, &mut txn).await);
    }
    txn.commit().await?;
    Ok(())
}
