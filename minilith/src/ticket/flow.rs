/// `user_ids` and `skip_activity_check` must be the same length. If no checks are to be done,
/// `skip_activity_check.len()` can be 0.
async fn reserve_user_purchase_flow(
    db: &mut Transaction<'_>,
    user_ids: &[String],
    ticket_kind: Uuid,
    skip_activity_check: &[bool],
) -> MinilithResult<()> {
    // This inserts them atomically so we don't get deadlocked if we'd do it one at a time and
    // another request does this at the same time
    sqlx::query!(
        "insert into users_in_purchase_flow (user_id, ticket_kind_id) select user_id, $2 as ticket_kind_id
        from unnest($1::text[]) as t(user_id)
        order by user_id",
        user_ids,
        ticket_kind
    )
    .execute(&mut db.executor())
    .await
    .map_err(|err| {
        if err
            .as_database_error()
            .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
        {
            MinilithEndpointError::bad_frontend_code(
                "please complete your current transaction flow \
                (or the target user's when transfering)",
                err,
            )
        } else {
            err.into()
        }
    })?;
    for (user_id, _) in user_ids
        .iter()
        .zip(skip_activity_check.iter().copied())
        .filter(|(_, skip)| !skip)
    {
        let has_purchased_ticket_for_activity = sqlx::query_scalar!(
            "select exists (
                select 1 from ticket_kinds kind
                inner join ticket_kinds all_kinds on all_kinds.activity_id = kind.activity_id
                inner join purchased_tickets pt on pt.ticket_kind_id = all_kinds.id
                where kind.id = $2 and owner_id = $1
            ) as \"exists!\"",
            user_id,
            ticket_kind
        )
        .fetch_one(&mut db.executor())
        .await?;
        if has_purchased_ticket_for_activity {
            return Err(MinilithEndpointError::bad_frontend_code(
                "cannot own two tickets to an activity!",
                "",
            ));
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PurchaseFlow {
    ReleaseQueue,
    ReservationQueue,
    Reservation,
}
impl PurchaseFlow {
    /// # Returns
    ///
    /// Ticket kind to fill.
    #[allow(
        clippy::too_many_lines,
        reason = "it's linear and one can fold the match arms in one's editor"
    )]
    async fn cancel<'a>(
        self,
        ctx: &'a ContextWrapper,
        user_id: &str,
        mut txn: Transaction<'a>,
    ) -> MinilithResult<(Transaction<'a>, Option<Uuid>)> {
        let to_reserve = match self {
            PurchaseFlow::Reservation => {
                let row = sqlx::query!(
                    "select ticket_kind_id, transaction_id
                    from ticket_reservations where user_id = $1
                    for update",
                    user_id,
                )
                .fetch_one(&mut txn.executor())
                .await?;

                // try to cancel transaction instead
                txn = if let Some(id) = row.transaction_id {
                    let lock_id = Uuid::new_v4();
                    attach_operation_lock_to_flow(&mut txn, user_id, lock_id).await?;
                    txn.commit().await?;

                    let resp = ctx
                        .transactions_post(format!("/v0/{id}/cancel"))
                        .send()
                        .await;
                    let mut txn = ctx.db.begin().await?;
                    wait_for_user_purchase_flow(&mut txn, user_id, None, Some(lock_id))
                        .await?
                        .wrap_err_bad_frontend(
                            "cancel took too long, flow gone, purchase complete",
                        )?;
                    detach_operation_lock_to_flow(&mut txn, user_id).await?;

                    match resp {
                        Ok(resp) => match resp.status() {
                            reqwest::StatusCode::NOT_FOUND => {
                                // it's already cancelled
                            }
                            reqwest::StatusCode::FORBIDDEN => {
                                txn.commit().await?;
                                return Err(MinilithEndpointError::bad_user_input(
                                    "tried to cancel when disallowed",
                                    id,
                                    "cannot cancel your current transaction at this point",
                                    "cancel",
                                ));
                            }
                            status if !status.is_success() => {
                                txn.commit().await?;
                                return Err(MinilithEndpointError::internal_error(
                                    "l1: transaction cancel failed!",
                                    resp.status(),
                                ));
                            }
                            _ => {}
                        },
                        Err(error) => {
                            txn.commit().await?;
                            return Err(MinilithEndpointError::internal_error(
                                "failed to cancel transaction due to connection issues",
                                error,
                            ));
                        }
                    }
                    txn
                    // transaction is cancelled
                } else {
                    txn
                };
                let affected = sqlx::query!(
                    "delete from ticket_reservations where user_id = $1",
                    user_id,
                )
                .execute(&mut txn.executor())
                .await?;
                ensure_affected_rows(
                    affected.rows_affected(),
                    1,
                    "reservation disappeared while dropping purchase flow",
                )?;
                sqlx::query!(
                    "update ticket_kinds
                    set reserved_or_purchased_tickets = reserved_or_purchased_tickets - 1
                    where id = $1",
                    row.ticket_kind_id,
                )
                .execute(&mut txn.executor())
                .await?;
                Some(row.ticket_kind_id)
            }
            PurchaseFlow::ReleaseQueue => {
                let affected = sqlx::query_scalar!(
                    "delete from ticket_release_queuers where user_id = $1",
                    user_id
                )
                .execute(&mut txn.executor())
                .await?;
                ensure_affected_rows(
                    affected.rows_affected(),
                    1,
                    "release queuer disappeared while dropping purchase flow",
                )?;
                None
            }
            PurchaseFlow::ReservationQueue => {
                let affected = sqlx::query_scalar!(
                    "delete from ticket_reservation_queuers where user_id = $1",
                    user_id
                )
                .execute(&mut txn.executor())
                .await?;
                ensure_affected_rows(
                    affected.rows_affected(),
                    1,
                    "reservation queuer disappeared while dropping purchase flow",
                )?;
                None
            }
        };
        Ok((txn, to_reserve))
    }
}

#[derive(Debug)]
struct PurchaseFlowWithKind {
    ticket_kind_id: Uuid,
    flow: PurchaseFlow,
}
impl Deref for PurchaseFlowWithKind {
    type Target = PurchaseFlow;
    fn deref(&self) -> &Self::Target {
        &self.flow
    }
}

#[derive(Debug)]
struct PurchaseFlowRow {
    ticket_kind_id: Uuid,
    reservation: Option<String>,
    release_queue: Option<String>,
    reservation_queue: Option<String>,
    lock_id: Option<Uuid>,
    locked_at: Option<OffsetDateTime>,
}

fn decode_purchase_flow(
    row: PurchaseFlowRow,
    ticket_kind: Option<Uuid>,
) -> MinilithResult<PurchaseFlowWithKind> {
    if ticket_kind.is_some_and(|ticket_kind| ticket_kind != row.ticket_kind_id) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "tried to continue purchase flow with different ticket kind",
            "",
        ));
    }
    let flow = match (
        row.release_queue.is_some(),
        row.reservation_queue.is_some(),
        row.reservation.is_some(),
    ) {
        (true, false, false) => PurchaseFlow::ReleaseQueue,
        (false, true, false) => PurchaseFlow::ReservationQueue,
        (false, false, true) => PurchaseFlow::Reservation,
        _ => {
            return Err(MinilithEndpointError::internal_error(
                "user has invalid purchase flow state",
                row,
            ));
        }
    };
    Ok(PurchaseFlowWithKind {
        ticket_kind_id: row.ticket_kind_id,
        flow,
    })
}
async fn process_purchase_flow(
    db: &mut Transaction<'_>,
    row: PurchaseFlowRow,
    ticket_kind: Option<Uuid>,
    user_id: &str,
    lock_id: Option<Uuid>,
) -> MinilithResult<PurchaseFlowWithKind> {
    if let Some(lock_id) = lock_id
        && row.lock_id != Some(lock_id)
    {
        return Err(MinilithEndpointError::bad_frontend_code(
            "we took too long, don't continue",
            row.lock_id,
        ));
    }
    if lock_id != row.lock_id && !user_id.is_empty() {
        match row.locked_at {
            None => {}
            Some(locked_at) if locked_at < OffsetDateTime::now_utc() - time::Duration::MINUTE => {
                // operation locks are only held for cancel, which means that in the worst case, the
                // cancel went through but we don't know of it, it'll just appear to us as if it got
                // cancelled from the external provider side

                // remove locked_at
                sqlx::query!(
                    "update users_in_purchase_flow set lock_id = null, locked_at = null
                    where user_id = $1",
                    user_id
                )
                .execute(&mut db.executor())
                .await?;
                error!("Removed lock because operation took more than 1minute!");
                alert(
                    AlertLevel::L3,
                    "Removed lock because operation took more than 30s!",
                );
            }
            Some(_) => {
                // a different op is holding a lock
                return Err(MinilithEndpointError::bad_frontend_code(
                    "another request is handling this flow or we took too long",
                    row.lock_id,
                ));
            }
        }
    }

    decode_purchase_flow(row, ticket_kind)
}

/// Can only be for cancel for now.
async fn attach_operation_lock_to_flow(
    db: &mut Transaction<'_>,
    user_id: &str,
    lock_id: Uuid,
) -> MinilithResult<()> {
    sqlx::query!(
        "update users_in_purchase_flow set lock_id = $1, locked_at = now()
        where user_id = $2",
        lock_id,
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    Ok(())
}
/// You need to call [`lock_user_purchase_flow`] directly before this or similar to validate the
/// `lock_id`.
async fn detach_operation_lock_to_flow(
    db: &mut Transaction<'_>,
    user_id: &str,
) -> MinilithResult<()> {
    sqlx::query!(
        "update users_in_purchase_flow set lock_id = null, locked_at = null
        where user_id = $1",
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    Ok(())
}
async fn lock_user_purchase_flow(
    db: &mut Transaction<'_>,
    user_id: &str,
    ticket_kind: Option<Uuid>,
    lock_id: Option<Uuid>,
) -> MinilithResult<PurchaseFlowWithKind> {
    let row = sqlx::query_as!(
        PurchaseFlowRow,
        "select ticket_kind_id, reservation, release_queue, reservation_queue,
        lock_id, locked_at
        from users_in_purchase_flow where user_id = $1 for update nowait",
        user_id
    )
    .fetch_optional(&mut db.executor())
    .await
    .map_err(|err| {
        if err
            .as_database_error()
            .is_some_and(|err| err.code().as_deref() == Some("55P03"))
        {
            MinilithEndpointError::bad_frontend_code(
                "another request is handling this purchase flow",
                err,
            )
        } else {
            err.into()
        }
    })?
    .wrap_err_not_found()?;
    process_purchase_flow(db, row, ticket_kind, user_id, lock_id).await
}

/// Internal jobs wait for the flow lock instead of dropping a transaction
/// callback or repeatedly selecting the same worker job.
async fn wait_for_user_purchase_flow(
    db: &mut Transaction<'_>,
    user_id: &str,
    ticket_kind: Option<Uuid>,
    lock_id: Option<Uuid>,
) -> MinilithResult<Option<PurchaseFlowWithKind>> {
    let row = sqlx::query_as!(
        PurchaseFlowRow,
        "select ticket_kind_id, reservation, release_queue, reservation_queue,
        lock_id, locked_at
        from users_in_purchase_flow where user_id = $1 for update",
        user_id
    )
    .fetch_optional(&mut db.executor())
    .await?;
    if let Some(row) = row {
        Ok(Some(
            process_purchase_flow(db, row, ticket_kind, user_id, lock_id).await?,
        ))
    } else {
        Ok(None)
    }
}
/// Internal jobs wait for the flow lock instead of dropping a transaction
/// callback or repeatedly selecting the same worker job.
///
/// This invalidates the current operation on this flow.
async fn invalidate_wait_for_user_purchase_flow_on_transaction_id(
    db: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<Option<PurchaseFlowWithKind>> {
    // Update locks the row
    let row = sqlx::query_as!(
        PurchaseFlowRow,
        "with selected_reservation as (
            select user_id from ticket_reservations
            where transaction_id = $1
        )
        update users_in_purchase_flow as flow
        set lock_id = null, locked_at = null
        from selected_reservation
        where flow.user_id = selected_reservation.user_id
        returning flow.ticket_kind_id, flow.reservation, flow.release_queue,
            flow.reservation_queue, flow.lock_id, flow.locked_at",
        transaction_id
    )
    .fetch_optional(&mut db.executor())
    .await?;
    row.map(|row| decode_purchase_flow(row, None)).transpose()
}
/// Internal jobs wait for the flow lock instead of dropping a transaction
/// callback or repeatedly selecting the same worker job.
async fn wait_for_user_purchase_flow_on_transaction_id(
    db: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<Option<PurchaseFlowWithKind>> {
    let row = sqlx::query_as!(
        PurchaseFlowRow,
        "with reservation as (
            select user_id from ticket_reservations
            where transaction_id = $1
        )
        select ticket_kind_id, reservation, release_queue, reservation_queue,
        lock_id, locked_at
        from reservation
        inner join users_in_purchase_flow flow on flow.user_id = reservation.user_id
        for update of flow",
        transaction_id
    )
    .fetch_optional(&mut db.executor())
    .await?;
    row.map(|row| decode_purchase_flow(row, None)).transpose()
}

async fn set_user_purchase_flow_release_queue(
    db: &mut Transaction<'_>,
    user_id: &str,
) -> MinilithResult<()> {
    let affected = sqlx::query!(
        "update users_in_purchase_flow
        set release_queue = $1
        where user_id = $1
        and release_queue is null
        and reservation_queue is null
        and reservation is null",
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to initialize release-queue purchase flow",
    )
}
async fn set_user_purchase_flow_reservation_queue(
    db: &mut Transaction<'_>,
    user_id: &str,
) -> MinilithResult<()> {
    let affected = sqlx::query!(
        "update users_in_purchase_flow
        set reservation_queue = $1
        where user_id = $1
        and release_queue is null
        and reservation_queue is null
        and reservation is null",
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to initialize reservation-queue purchase flow",
    )
}
async fn set_user_purchase_flow_reservation(
    db: &mut Transaction<'_>,
    user_id: &str,
) -> MinilithResult<()> {
    let affected = sqlx::query!(
        "update users_in_purchase_flow
        set reservation = $1
        where user_id = $1
        and release_queue is null
        and reservation_queue is null
        and reservation is null",
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "failed to initialize reservation purchase flow",
    )
}
async fn unlist_user_purchase_flow(
    db: &mut Transaction<'_>,
    user_id: impl Into<String>,
) -> MinilithResult<()> {
    unlist_users_purchase_flow(db, &[user_id.into()]).await
}

async fn unlist_users_purchase_flow(
    db: &mut Transaction<'_>,
    user_ids: &[String],
) -> MinilithResult<()> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let affected = sqlx::query!(
        "delete from users_in_purchase_flow where user_id = any($1)",
        user_ids
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        user_ids.len(),
        "failed to finish multiple purchase flows",
    )
}
