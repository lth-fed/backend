/// # Panics
///
/// Never.
fn new_timeout_interval() -> PgInterval {
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

/// Atomically reserves up to `requested` places while enforcing both the
/// ticket-kind and activity-wide limits. Locking the activity row serializes
/// reservations made concurrently for different ticket kinds of the same
/// activity.
async fn reserve_ticket_capacity(
    db: &mut Transaction<'_>,
    ticket_kind: Uuid,
    requested: i32,
) -> MinilithResult<i32> {
    // This must be a separate statement from the SUM below. At READ COMMITTED,
    // a statement which waits for a row lock keeps its original statement
    // snapshot; the next statement gets a fresh snapshot after the previous
    // activity reservation committed.
    let activity_exists = sqlx::query_scalar!(
        r#"select activities.id
        from activities
        inner join ticket_kinds target
            on target.activity_id = activities.id
        where target.id = $1
        for update of activities"#,
        ticket_kind,
    )
    .fetch_optional(&mut db.executor())
    .await?
    .is_some();
    if !activity_exists {
        return Ok(0);
    }

    let granted = sqlx::query_scalar!(
        r#"with capacity as materialized (
            select greatest(least(
                $2,
                kind.max_tickets - kind.reserved_or_purchased_tickets,
                activities.max_tickets
                - kind.reserved_or_purchased_tickets
                - coalesce((
                    select sum(greatest(
                        all_kinds.reserved_or_purchased_tickets,
                        all_kinds.min_tickets
                    ))::int
                    from ticket_kinds all_kinds
                    where all_kinds.activity_id = activities.id
                        and all_kinds.id != kind.id
                ), 0)
            ), 0)::int as granted
            from ticket_kinds kind
            inner join activities on activities.id = kind.activity_id
            where kind.id = $1
        ), updated as (
            update ticket_kinds
            set reserved_or_purchased_tickets =
                reserved_or_purchased_tickets + capacity.granted
            from capacity
            where ticket_kinds.id = $1
            returning ticket_kinds.id
        )
        select capacity.granted
        from capacity
        inner join updated on true"#,
        ticket_kind,
        requested,
    )
    .fetch_optional(&mut db.executor())
    .await?
    .flatten()
    .unwrap_or(0);
    Ok(granted)
}

/// Checks for reservations which are timed out. If any is found, it's removed.
/// Call [`give_reservations`] after calling this.
///
/// # Errors
///
/// Failures from cancelling transactions.
pub async fn remove_reservation(db: &PgPool) -> MinilithResult<ControlFlow<()>> {
    let mut txn = db.begin().await?;
    // Lock the flow first; child reservation rows are always locked second.
    let removed_reservation = sqlx::query!(
        "select flow.user_id, flow.ticket_kind_id
        from users_in_purchase_flow flow
        inner join ticket_reservations reservation
            on reservation.user_id = flow.user_id
            and reservation.ticket_kind_id = flow.ticket_kind_id
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
            "delete from ticket_reservations
            where user_id = $1
            and ticket_kind_id = $2
            and timeout < now()
            and transaction_id is null",
            reservation.user_id,
            reservation.ticket_kind_id,
        )
        .execute(&mut txn.executor())
        .await?;
        ensure_affected_rows(
            affected.rows_affected(),
            1,
            "expired reservation changed after its flow was locked",
        )?;
        sqlx::query!(
            "update ticket_kinds
            set reserved_or_purchased_tickets = reserved_or_purchased_tickets - 1
            where id = $1",
            reservation.ticket_kind_id,
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
/// If there are reservation spots left, a person from the `reservation_queue` will get a reservation.
///
/// Also handles the case where there's a stray in the reservation queue.
///
/// # Errors
///
/// None:) only db.
pub async fn give_reservations(
    ticket_kind: Uuid,
    fetch_n: i32,
    db: &mut Transaction<'_>,
) -> MinilithResult<()> {
    let mut new_reservations = sqlx::query_scalar!(
        "select flow.user_id
        from users_in_purchase_flow flow
        inner join ticket_reservation_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.ticket_kind_id = flow.ticket_kind_id
        where flow.ticket_kind_id = $1
        and flow.reservation_queue = flow.user_id
        order by queuer.placement asc, flow.user_id
        limit $2
        for update of flow skip locked",
        ticket_kind,
        i64::from(fetch_n)
    )
    .fetch_all(&mut db.executor())
    .await?;
    if new_reservations.is_empty() {
        return Ok(());
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we'll never get this high"
    )]
    let granted = reserve_ticket_capacity(db, ticket_kind, new_reservations.len() as i32).await?;

    #[allow(clippy::cast_sign_loss, reason = "removed will always be positive")]
    new_reservations.truncate(granted as usize);
    if new_reservations.is_empty() {
        return Ok(());
    }
    let timestamps: Vec<PgInterval> = std::iter::repeat_with(new_timeout_interval)
        .take(new_reservations.len())
        .collect();
    let affected = sqlx::query!(
        "insert into ticket_reservations (user_id, ticket_kind_id, transaction_id, timeout)
        select user_id, $2 as ticket_kind_id, null as transaction_id,
        (from_now + now()) as timeout
        from unnest($1::text[], $3::interval[]) as t(user_id, from_now)",
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
        and ticket_kind_id = $2
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
        "delete from ticket_reservation_queuers
        where ticket_kind_id = $1
        and user_id = any($2)",
        ticket_kind,
        &new_reservations
    )
    .execute(&mut db.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        new_reservations.len(),
        "failed to remove promoted reservation queuers",
    )
}

async fn give_reservations_in_new_transaction(
    db: &PgPool,
    ticket_kind: Uuid,
    fetch_n: i32,
) -> MinilithResult<()> {
    let mut txn = db.begin().await?;
    give_reservations(ticket_kind, fetch_n, &mut txn).await?;
    txn.commit().await?;
    Ok(())
}
/// Clear reservation queue when there are no more tickets.
/// We use `purchased_tickets` since they never decrease so the lock on it doesn't matter!
async fn remove_queuers_when_sold_out(db: &mut Transaction<'_>) -> MinilithResult<()> {
    let queuers = sqlx::query_scalar!(
        r#"select flow.user_id
        from users_in_purchase_flow flow
        inner join ticket_reservation_queuers queuer
            on queuer.user_id = flow.user_id
            and queuer.ticket_kind_id = flow.ticket_kind_id
        inner join ticket_kinds kind on kind.id = queuer.ticket_kind_id
        where flow.reservation_queue = flow.user_id
        and
        (
            -- inidvidual ticket
            (
                kind.max_tickets = kind.reserved_or_purchased_tickets
                and (
                    select count(*) from purchased_tickets
                    where ticket_kind_id = kind.id
                ) >= kind.max_tickets
            )
            -- other ticket_kinds
            -- this pathway should not often be reached since ticket kinds often are not released simultaneously
            or exists (
                select 1
                from activities
                where activities.id = kind.activity_id
                and (
                    select count(*)
                    from purchased_tickets
                    inner join ticket_kinds purchased_kind
                        on purchased_kind.id = purchased_tickets.ticket_kind_id
                    where purchased_kind.activity_id = activities.id
                ) >= activities.max_tickets
            )
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
        "delete from ticket_reservation_queuers where user_id = any($1)",
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
