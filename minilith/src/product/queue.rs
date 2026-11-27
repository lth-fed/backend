use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use sqlx::types::time::OffsetDateTime;

use super::{
    access::ensure_user_may_purchase_ticket,
    allocation::{
        give_reservations, is_invited_buyer, new_timeout_interval, queue_base,
        reserve_ticket_capacity,
    },
    ensure_affected_rows,
    flow::{
        PurchaseFlow, lock_user_purchase_flow, reserve_user_purchase_flow,
        set_user_purchase_flow_release_queue, set_user_purchase_flow_reservation,
        set_user_purchase_flow_reservation_queue, unlist_user_purchase_flow,
        wait_for_user_purchase_flow,
    },
    models::{PurchaseStatus, QueueRequest, QueueResponse},
    notifications::{TicketNotification, notify_ticket_users},
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
    // Only the activity's base ticket has a reservation queue; everybody else queues there and
    // remembers the kind they want in `original_product_id`.
    let queue_kind = queue_base(&mut txn.executor(), req.ticket_kind).await?;
    // TODO(frontend-hack: 25/08/2026): frontend doesn't send a DELETE to /queue so we delete it here in case the user
    // wants to queue for a different ticket kind.
    match lock_user_purchase_flow(&mut txn, user.get_id(), None, None).await {
        Ok(flow) => {
            let queued_for = sqlx::query_scalar!(
                r#"select coalesce(original_product_id, product_id) as "wanted!"
                from product_reservation_queuers where user_id = $1"#,
                user.get_id()
            )
            .fetch_optional(&mut txn.executor())
            .await?;
            if queued_for.unwrap_or_else(|| flow.product_id()) != req.ticket_kind {
                let (mut txn, ticket_kind_to_fill) = flow.cancel(ctx, user.get_id(), txn).await?;
                unlist_user_purchase_flow(&mut txn, user.get_id()).await?;

                let promoted = if let Some(ticket_kind) = ticket_kind_to_fill {
                    give_reservations(ticket_kind, 1, &mut txn).await?
                } else {
                    Vec::new()
                };

                txn.commit().await?;
                if let Some(ticket_kind) = ticket_kind_to_fill {
                    notify_ticket_users(
                        ctx,
                        ticket_kind,
                        promoted,
                        TicketNotification::Reservation,
                    );
                }
                return Err(MinilithEndpointError::bad_frontend_code(
                    "your current flow is cancelled, press the button again to place in this queue",
                    "",
                ));
            }
        }
        Err(MinilithEndpointError::NotFound(_)) => {}
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
        "select tickets_been_released, sales_stop from products where id = $1",
        req.ticket_kind
    )
    .fetch_one(&mut txn.executor())
    .await?;

    if row.sales_stop < OffsetDateTime::now_utc() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "ticket not available for purchase anymore",
            "",
        ));
    }

    if row.tickets_been_released {
        let row = sqlx::query!(
            "select (
                    select count(user_id) from product_reservation_queuers
                    where product_id = $1 or original_product_id = $1
                ) as \"count!\"",
            req.ticket_kind
        )
        .fetch_one(&mut txn.executor())
        .await?;

        // An invited person may use the unit promised to them; everybody else cannot.
        let invited = is_invited_buyer(&mut txn.executor(), req.ticket_kind, user.get_id()).await?;
        let reserved = reserve_ticket_capacity(&mut txn, req.ticket_kind, 1, !invited).await?;
        if reserved == 0 {
            return Err(MinilithEndpointError::bad_user_input(
                "the tickets are sold out",
                "",
                "the tickets are sold out",
                "product_id",
            ));
        }
        if row.count == 0 && reserved == 1 {
            // give reservation
            let affected = sqlx::query!(
                "insert into product_reservations
                        (user_id, product_id, product_price, transaction_id, timeout)
                        select $1, $2, base.price, null, now() + $3
                        from products kind
                        inner join product_base_prices base on base.product_id = kind.id
                        where kind.id = $2",
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

        // The queue position belongs to the base ticket; the flow has to point there first
        // (the flow → queue foreign key is immediate).
        let affected = sqlx::query!(
            "update users_in_purchase_flow
                set product_id = $2
            where user_id = $1
            and product_id = $3
            and release_queue is null
            and reservation_queue is null
            and reservation is null",
            user.get_id(),
            queue_kind,
            req.ticket_kind
        )
        .execute(&mut txn.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            1,
            "failed to move the purchase flow to the base ticket queue",
        )?;
        let wanted = (queue_kind != req.ticket_kind).then_some(req.ticket_kind);
        let affected = sqlx::query!(
            "with placement as (
                    update product_reservation_placement_tails
                        set placement_tail = placement_tail + 1
                    where product_id = $2
                    returning old.placement_tail
                )
                insert into product_reservation_queuers
                    (user_id, product_id, original_product_id, placement)
                select $1, $2, $3, placement.placement_tail
                from placement

                on conflict (user_id) do update
                set product_id = excluded.product_id,
                    original_product_id = excluded.original_product_id,
                    placement = excluded.placement",
            user.get_id(),
            queue_kind,
            wanted
        )
        .execute(&mut txn.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            1,
            "failed to enter reservation queue",
        )?;
        // We took a unit above but are queuing behind others, so give it back: the promotion
        // pass hands it to the head of the queue instead.
        sqlx::query!(
            "update products
            set sales_stock = sales_stock + 1
            where id = $1 and sales_stock is not null",
            req.ticket_kind
        )
        .execute(&mut txn.executor())
        .await?;
        set_user_purchase_flow_reservation_queue(&mut txn, user.get_id()).await?;
        txn.commit().await?;
        Ok(PurchaseStatus::ReservationQueued)
    } else {
        let affected = sqlx::query!(
            "insert into product_release_queuers (user_id, product_id, started_queueing) \
                select $1, $2, now()
                where not exists (
                    select 1 from purchases where product_id = $2 and owner_id = $1
                )
                on conflict (user_id) do update
                set product_id = excluded.product_id,
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
                "select product_id, timeout
                    from product_reservations
                    where user_id = $1",
                user.get_id()
            )
            .fetch_one(&mut txn.executor())
            .await?;
            Ok(QueueResponse {
                ticket_kind: reservation.product_id,
                placement: Some(0),
                timeout: Some(reservation.timeout),
                start_transaction_before: Some(reservation.timeout - 1 * time::Duration::MINUTE),
            })
        }
        PurchaseFlow::ReservationQueue => {
            let reservation_queue = sqlx::query!(
                r#"select placement, coalesce(original_product_id, product_id) as "product_id!"
                    from product_reservation_queuers
                    where user_id = $1"#,
                user.get_id()
            )
            .fetch_one(&mut txn.executor())
            .await?;
            Ok(QueueResponse {
                ticket_kind: reservation_queue.product_id,
                placement: Some(reservation_queue.placement),
                timeout: None,
                start_transaction_before: None,
            })
        }
        PurchaseFlow::ReleaseQueue => {
            let queuer = sqlx::query_scalar!(
                "select product_id from product_release_queuers where user_id = $1",
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

    let promoted = if let Some(ticket_kind) = ticket_kind_to_fill {
        give_reservations(ticket_kind, 1, &mut txn).await?
    } else {
        Vec::new()
    };
    txn.commit().await?;
    if let Some(ticket_kind) = ticket_kind_to_fill {
        notify_ticket_users(ctx, ticket_kind, promoted, TicketNotification::Reservation);
    }
    Ok(())
}
