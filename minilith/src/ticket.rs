use std::collections::{HashMap, HashSet};
use std::ops::{ControlFlow, Deref};

use bin_common::{PgPool, Transaction};
use fed_auth_verifier::User;
use fed_auth_verifier::callbacks::TransactionState;
use poem_openapi::Enum;
use poem_openapi::{Object, OpenApi, payload::Json};
use rand::seq::SliceRandom as _;
use rand::{random, rng};
use sqlx::PgExecutor;
use sqlx::postgres::types::PgInterval;
use sqlx::types::time::OffsetDateTime;
use tracing::error;
use uuid::Uuid;

use crate::activities::{Location, PoemLocation};
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, InternationalizedString as IS,
    MinilithEndpointError, MinilithErrorResultExt as _, MinilithResult,
};

#[derive(Debug, Clone)]
pub struct Router {
    pub context: ContextWrapper,
}

impl Deref for Router {
    type Target = ContextWrapper;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Debug, Clone, Object)]
pub struct GetFreeTicketRequest {
    ticket_kind: Uuid,
    addons: Vec<Uuid>,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct QueueRequest {
    ticket_kind: Uuid,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct UnqueueRequest {
    ticket_kind: Uuid,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct QueueResponse {
    ticket_kind: Uuid,
    /// A placement of 0 indicates you can buy the ticket.
    /// `None` indicates the tickets have not yet been released.
    placement: Option<i32>,
    /// When the ticket will be made unavailable for purchase, i.e. the reservation ran out.
    /// Will be not null when placement is 0.
    timeout: Option<OffsetDateTime>,
    /// When transactions at latest should be conducted.
    /// Will be not null when placement is 0.
    latest_transaction: Option<OffsetDateTime>,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct PurchaseStatusRequest {
    activity: Uuid,
}

#[derive(Debug, Clone, Copy, Enum)]
pub enum DropStatus {
    Dropped,
    /// Transaction is in the state of being cancelled. Poll every ~5s the status of the queue to
    /// see when it has been successfully cancelled.
    /// The transaction may still go through if you have accepted payment.
    TransactionCancelling,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct DropReservationResponse {
    status: DropStatus,
}

#[derive(Debug, Clone, Copy, Enum)]
pub enum PurchaseStatus {
    /// Standing in release queue (tickets have not been released yet).
    /// Request the queue endpoint to get queue status.
    ReleaseQueued,
    /// Standing in reservation queue (tickets have been released).
    /// Request the queue endpoint to get queue status.
    ReservationQueued,
    /// Ready to be transacted.
    Reserved,
    /// Transaction is happening. Making another transaction request will override the current
    /// transaction.
    Buying,
    /// User owns the ticket now.
    Purchased,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct PurchaseStatusResponse {
    ticket_kind: Option<Uuid>,
    status: PurchaseStatus,
}

#[derive(Object)]
struct PurchasedAddon {
    idx: i32,
    multiple_alternatives: bool,
    has_text_field: bool,
    required: bool,
    selected_options: Vec<i32>,
    selected_text: String,
}

#[derive(Object)]
struct Ticket {
    id: Uuid,
    #[allow(clippy::struct_field_names, reason = "reasonable name")]
    ticket_kind_id: Uuid,
    activity_id: Uuid,
    #[allow(clippy::struct_field_names, reason = "reasonable name")]
    ticket_kind_name: IS,
    activity_location: PoemLocation,
    activity_title: IS,
    creator_id: Uuid,
    creator_path: String,
    creator_name: IS,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    addons: Vec<PurchasedAddon>,
}
#[derive(Object)]
struct PurchasedTicket {
    id: Uuid,
}

#[OpenApi(prefix_path = "/tickets")]
impl Router {
    /// # Errors
    ///
    /// AUTH, DB
    #[oai(path = "/", method = "get")]
    async fn my_tickets(&self, user: User) -> MinilithResult<Json<Vec<Ticket>>> {
        let id = user.get_id();

        let mut addons: HashMap<Uuid, Vec<PurchasedAddon>> = sqlx::query!(
            r#"select
                purchased_ticket_addons.ticket_id as "ticket_id",
                ticket_addons.idx as "idx",
                ticket_addons.multiple_alternatives as "multiple_alternatives",
                ticket_addons.has_text_field as "has_text_field",
                ticket_addons.required as "required",
                purchased_ticket_addons.selected_options as "selected_options",
                purchased_ticket_addons.selected_text as "selected_text"
            from purchased_tickets
            inner join purchased_ticket_addons on purchased_ticket_addons.ticket_id = purchased_tickets.id
            inner join ticket_addons on ticket_addons.id = purchased_ticket_addons.addon_id
            where purchased_tickets.owner_id = $1
            order by ticket_addons.idx
            "#,
            id
        )
        .map(|row| {
            (
                row.ticket_id,
                PurchasedAddon {
                    idx: row.idx,
                    multiple_alternatives: row.multiple_alternatives,
                    has_text_field: row.has_text_field,
                    required: row.required,
                    selected_options: row.selected_options,
                    selected_text: row.selected_text,
                },
            )
        })
        .fetch_all(&self.context.db)
        .await
        .wrap_err_db()?
        .into_iter()
        .fold(HashMap::new(), |mut map, (ticket_id, addon)| {
            map.entry(ticket_id).or_default().push(addon);
            map
        });

        let tickets = sqlx::query!(
            r#"select
                purchased_tickets.id as "id",
                purchased_tickets.ticket_kind_id as "ticket_kind_id",
                ticket_kinds.activity_id as "activity_id",
                ticket_kinds.name as "ticket_kind_name!: DIS",
                activities.title as "activity_title!: DIS",
                creator.id as creator_id,
                creator.path as creator_path,
                creator.name as "creator_name!: DIS",
                activities.location as "location!: Location",
                activities.time_start as "time_start",
                activities.time_end as "time_end"
            from purchased_tickets
            inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
            inner join activities on activities.id = ticket_kinds.activity_id
            inner join groups creator on creator.id = activities.creator_id
            where purchased_tickets.owner_id = $1
            "#,
            id
        )
        .map(|ticket| Ticket {
            addons: addons.remove(&ticket.id).unwrap_or_default(),
            id: ticket.id,
            ticket_kind_id: ticket.ticket_kind_id,
            activity_id: ticket.activity_id,
            activity_location: ticket.location.into(),
            activity_title: ticket.activity_title.0,
            ticket_kind_name: ticket.ticket_kind_name.0,
            creator_id: ticket.creator_id,
            creator_path: ticket.creator_path.to_string(),
            creator_name: ticket.creator_name.0,
            time_start: ticket.time_start,
            time_end: ticket.time_end,
        })
        .fetch_all(&self.context.db)
        .await
        .wrap_err_db()?;

        Ok(Json(tickets))
    }

    /// # Errors
    ///
    /// DB, AUTH, `BF_TKS_VALD_ADDON_DUPLIC` (duplicate addon), `BF_TKS_VALD_ADDON_NR` (not all
    /// addons are sent), `BU_TKS_VALD_PURCH` (user already owns a ticket from this event).
    #[oai(path = "/", method = "post")]
    async fn get_free_ticket(
        &self,
        user: User,
        req: Json<GetFreeTicketRequest>,
    ) -> MinilithResult<Json<PurchasedTicket>> {
        let mut txn = self.db.begin().await.wrap_err_db()?;

        validate_addons(&mut txn.executor(), &req.addons, req.ticket_kind).await?;
        ensure_user_may_purchase_ticket(&mut txn.executor(), &user, req.ticket_kind).await?;

        let ticket_id = sqlx::query_scalar!(
            "insert into purchased_tickets (ticket_kind_id, purchaser_id, owner_id) values ($1, $2, $2) returning id",
            req.ticket_kind,
            user.get_id(),
        )
        .fetch_one(&mut txn.executor())
        .await
        .wrap_err_db()?;

        for addon in &req.addons {
            sqlx::query!(
                "insert into purchased_ticket_addons (addon_id, ticket_id) values ($1, $2)",
                addon,
                ticket_id
            )
            .execute(&mut txn.executor())
            .await
            .wrap_err_db()?;
        }

        // increment ticket_kinds.reserved_or_purchased_tickets and set
        // has_been_purchased to true
        sqlx::query!(
            "update ticket_kinds set reserved_or_purchased_tickets = reserved_or_purchased_tickets + 1, has_been_purchased = true where id = $1",
            req.ticket_kind
        )
        .execute(&mut txn.executor())
        .await
            .wrap_err_db()?;

        txn.commit().await.wrap_err_db()?;

        Ok(Json(PurchasedTicket { id: ticket_id }))
    }

    /// Places the user in the queue for this `ticket_kind`.
    /// - if queue response is queued, get queue status & display wait &
    ///   (if reservation queue: refresh every 15 seconds, else refresh after the ticket is released)
    /// - if queue response is reserved, go to buy
    /// - (runtime releases tickets when it's time)
    ///
    /// You have to call this once every 15 minutes since it removes the queue spot after 20
    /// minutes.
    ///
    /// # Errors
    ///
    /// None
    #[oai(path = "/queue", method = "put")]
    async fn queue(
        &self,
        user: User,
        req: Json<QueueRequest>,
    ) -> MinilithResult<Json<PurchaseStatus>> {
        let has_been_released = sqlx::query_scalar!(
            "select has_been_released from ticket_kinds where id = $1",
            req.ticket_kind
        )
        .fetch_one(&self.db)
        .await
        .wrap_err_db()?;

        if has_been_released {
            // we can only be in 1 type of queue
            let _: Result<_, _> = self.unqueue(user.clone()).await;
            let row = sqlx::query!(
                "select
                (select count(user_id) from ticket_reservation_queuers
                 where ticket_kind_id = $1) as \"count!\",
                reserved_or_purchased_tickets, max_tickets
                from ticket_kinds where id = $1",
                req.ticket_kind
            )
            .fetch_one(&self.db)
            .await
            .wrap_err_db()?;

            if row.reserved_or_purchased_tickets < row.max_tickets && row.count == 0 {
                let mut txn = self.db.begin().await.wrap_err_db()?;
                // will fail if we've reserved too many
                if sqlx::query!(
                    "update ticket_kinds set
                    reserved_or_purchased_tickets = reserved_or_purchased_tickets + 1"
                )
                .execute(&mut txn.executor())
                .await
                .is_ok()
                {
                    // give reservation
                    sqlx::query!(
                    "insert into ticket_reservations (user_id, ticket_kind_id, transaction_id, timeout)
                    values ($1, $2, null, now() + $3)",
                    user.get_id(),
                    req.ticket_kind,
                    new_timeout_interval()
                )
                    .execute(&mut txn.executor())
                    .await
                    .wrap_err_db()?;
                    if txn.commit().await.is_ok() {
                        return Ok(Json(PurchaseStatus::Reserved));
                    }
                }
                // if the txn fails, it's because we've tried to reserve too many, stand in queue
                // instead:
            }

            sqlx::query!(
                "insert into ticket_reservation_queuers (user_id, ticket_kind_id, placement) \
                values ($1, $2,
                    -- take last placement, add one
                    (select placement
                     from ticket_reservation_queuers
                     order by placement desc limit 1) + 1
                )
                on conflict (user_id) do update
                set ticket_kind_id = excluded.ticket_kind_id, placement = excluded.placement",
                user.get_id(),
                req.ticket_kind
            )
            .execute(&self.db)
            .await
            .wrap_err_db()?;
            // if has_been_released, move to reservation queue / reservation
            // there's an edge-case here where this is inserted into the queue since from above it has
            // not been released. But then it released between there and here.
            // `update_misplaced_queuer` handles that.
            Ok(Json(PurchaseStatus::ReservationQueued))
        } else {
            // we can only be in 1 type of queue
            let _: Result<_, _> = self.drop_reservation(user.clone()).await;
            sqlx::query!(
                "insert into ticket_release_queuers (user_id, ticket_kind_id, started_queueing) \
                values ($1, $2, now())
                on conflict (user_id) do update
                set ticket_kind_id = excluded.ticket_kind_id, started_queueing = excluded.started_queueing",
                user.get_id(),
                req.ticket_kind
            )
            .execute(&self.db)
            .await
            .wrap_err_db()?;
            Ok(Json(PurchaseStatus::ReleaseQueued))
        }
    }
    /// Does not release a reservation.
    ///
    /// Call this if you no longer want to stay in a queue.
    ///
    /// # Errors
    ///
    /// - not found when the user is not release queued
    #[oai(path = "/queue", method = "delete")]
    async fn unqueue(&self, user: User) -> MinilithResult<()> {
        let rows = sqlx::query_scalar!(
            "delete from ticket_release_queuers where user_id = $1",
            user.get_id()
        )
        .execute(&self.db)
        .await
        .wrap_err_db()?;
        if rows.rows_affected() == 0 {
            Err(MinilithEndpointError::not_found())
        } else {
            Ok(())
        }
    }
    /// Get the status of the queue.
    ///
    /// # Errors
    ///
    /// - 404 not found when the user is not queued (neither reservation queue nor release queue)
    #[oai(path = "/queue", method = "get")]
    async fn queue_status(&self, user: User) -> MinilithResult<Json<QueueResponse>> {
        let reservation = sqlx::query!(
            "select ticket_kind_id, timeout
            from ticket_reservations
            where user_id = $1",
            user.get_id()
        )
        .fetch_optional(&self.db)
        .await
        .wrap_err_db()?;
        if let Some(row) = reservation {
            return Ok(Json(QueueResponse {
                ticket_kind: row.ticket_kind_id,
                placement: Some(0),
                timeout: Some(row.timeout),
                latest_transaction: Some(row.timeout - 1 * time::Duration::MINUTE),
            }));
        }
        let reservation_queue = sqlx::query!(
            "select placement, reserved_or_purchased_tickets, ticket_kind_id
            from ticket_reservation_queuers
            inner join ticket_kinds on (ticket_kinds.id = ticket_reservation_queuers.ticket_kind_id)
            where user_id = $1",
            user.get_id()
        )
        .fetch_optional(&self.db)
        .await
        .wrap_err_db()?;
        if let Some(row) = reservation_queue {
            return Ok(Json(QueueResponse {
                ticket_kind: row.ticket_kind_id,
                placement: Some((row.placement - row.reserved_or_purchased_tickets).max(0)),
                timeout: None,
                latest_transaction: None,
            }));
        }
        let queuer = sqlx::query_scalar!(
            "select ticket_kind_id from ticket_release_queuers where user_id = $1",
            user.get_id()
        )
        .fetch_optional(&self.db)
        .await
        .wrap_err_db()?;
        if let Some(id) = queuer {
            return Ok(Json(QueueResponse {
                ticket_kind: id,
                placement: None,
                timeout: None,
                latest_transaction: None,
            }));
        }
        Err(MinilithEndpointError::not_found())
    }
    /// Cancel the reservation if the user is no longer interested in buying it (e.g. realize they
    /// are broke).
    ///
    /// # Errors
    ///
    /// - 404 not found when the user doesn't have a reservation
    #[oai(path = "/reservation", method = "delete")]
    async fn drop_reservation(&self, user: User) -> MinilithResult<Json<DropReservationResponse>> {
        let mut txn = self.db.begin().await.wrap_err_db()?;
        let Some(row) = sqlx::query!(
            "delete from ticket_reservations where user_id = $1
            returning ticket_kind_id, transaction_id",
            user.get_id(),
        )
        .fetch_optional(&mut txn.executor())
        .await
        .wrap_err_db()?
        else {
            return Err(MinilithEndpointError::not_found());
        };
        // try to cancel transaction instead
        if let Some(id) = row.transaction_id {
            let resp = match self
                .transactions_post(format!("/v0/{id}/cancel"))
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    // ALERT LEVEL 2
                    error!(
                        ?err,
                        "failed to cancel transaction due to connection issues"
                    );
                    return Ok(Json(DropReservationResponse {
                        status: DropStatus::TransactionCancelling,
                    }));
                }
            };
            if !resp.status().is_success() {
                // ALERT LEVEL 1
                error!(
                    status_code=%resp.status(),
                    "transaction cancel failed!"
                );
            }
            return Ok(Json(DropReservationResponse {
                status: DropStatus::TransactionCancelling,
            }));
        }
        sqlx::query!(
            "update ticket_kinds
            set reserved_or_purchased_tickets = reserved_or_purchased_tickets - 1
            where id = $1",
            row.ticket_kind_id,
        )
        .execute(&mut txn.executor())
        .await
        .wrap_err_db()?;
        txn.commit().await.wrap_err_db()?;
        let mut txn = self.db.begin().await.wrap_err_db()?;
        give_reservations(row.ticket_kind_id, 1, &mut txn).await?;
        txn.commit().await.wrap_err_db()?;
        Ok(Json(DropReservationResponse {
            status: DropStatus::Dropped,
        }))
    }

    /// `/v0/tickets/callback`
    ///
    /// # Errors
    ///
    /// DB errors.
    #[oai(path = "/callback", method = "post")]
    pub async fn callback(
        &self,
        events: fed_auth_verifier::callbacks::TransactionsCallbackDataV1,
    ) -> MinilithResult<()> {
        for data in &*events {
            match data.inner.state {
                TransactionState::Pending => {}
                TransactionState::Paid => {
                    let mut txn = self.db.begin().await.wrap_err_db()?;
                    let affected = sqlx::query!(
                        "insert into purchased_tickets
                        (purchaser_id, owner_id, ticket_kind_id, transaction_id)
                        select user_id as purchaser_id, user_id as owner_id,
                        ticket_kind_id, transaction_id
                        from ticket_reservations
                            where transaction_id = $1",
                        data.transaction_id
                    )
                    .execute(&mut txn.executor())
                    .await
                    .wrap_err_db()?;
                    // has this already been marked as purchased?
                    if affected.rows_affected() != 1 {
                        let exists_purchased_ticket = sqlx::query_scalar!(
                            "select exists (
                                select 1 from purchased_tickets where transaction_id = $1
                            ) as \"exists!\"",
                            data.transaction_id
                        )
                        .fetch_one(&mut txn.executor())
                        .await
                        .wrap_err_db()?;
                        if !exists_purchased_ticket {
                            // ono somebody paid for a non-existing ticket!!
                            error!(transaction_id = %data.transaction_id,
                                "tried to pay for an unaccounted-for ticket"
                            );
                            // ALERT LEVEL 1
                        }
                        // otherwise, we're golden, this is just a second "person has paid" callback.
                        txn.rollback().await.wrap_err_db()?;
                        continue;
                    }
                    let affected = sqlx::query!(
                        "delete from ticket_reservations where transaction_id = $1",
                        data.transaction_id
                    )
                    .execute(&mut txn.executor())
                    .await
                    .wrap_err_db()?;
                    if affected.rows_affected() != 1 {
                        error!(transaction_id = %data.transaction_id,
                            "1 row not affected when purchase complete!"
                        );
                        // ALERT LEVEL 1
                        txn.rollback().await.wrap_err_db()?;
                        continue;
                    }
                    txn.commit().await.wrap_err_db()?;
                }
                TransactionState::Refunded => {
                    let affected = sqlx::query!(
                        "update purchased_tickets set owner_id = 'refunded:'
                    where transaction_id = $1",
                        data.transaction_id
                    )
                    .execute(&self.db)
                    .await
                    .wrap_err_db()?;
                    if affected.rows_affected() != 1 {
                        error!(transaction_id=%data.transaction_id,
                            "1 row not affected when purchase refunded!"
                        );
                        // ALERT LEVEL 1
                    }
                }
                TransactionState::Cancelled => {
                    let mut txn = self.db.begin().await.wrap_err_db()?;
                    let Some(row) = sqlx::query!(
                        "update ticket_reservations
                        set transaction_id = null 
                        returning id, timeout < now() as \"has_timed_out!\""
                    )
                    .fetch_optional(&mut txn.executor())
                    .await
                    .wrap_err_db()?
                    else {
                        error!(
                            transaction_id = %data.transaction_id,
                            "transaction which we do not track is cancelled"
                        );
                        // ALERT LEVEL 2
                        continue;
                    };
                    if row.has_timed_out {
                        sqlx::query!("delete from ticket_reservations where id = $1", row.id)
                            .execute(&mut txn.executor())
                            .await
                            .wrap_err_db()?;
                    }
                    txn.commit().await.wrap_err_db()?;
                }
            }
        }
        Ok(())
    }
}
/// # Panics
///
/// Never.
#[allow(clippy::unwrap_used, reason = "we know the interval is within range")]
fn new_timeout_interval() -> PgInterval {
    PgInterval::try_from(
        time::Duration::MINUTE * 15f64 + time::Duration::MINUTE * 3f64 * random::<f64>(),
    )
    .unwrap()
}
/// Releases a ticket. This MUST be called at the moment the tickets should be released.
///
/// # Errors
///
/// Db errors or if the id doesn't exist.
pub async fn release(db: &mut Transaction<'_>, id: Uuid) -> MinilithResult<()> {
    let ticket_kind = sqlx::query!(
        "select *
        from ticket_kinds 
        where id = $1",
        id
    )
    .fetch_one(&mut db.executor())
    .await
    .wrap_err_db()?;

    let mut queuers = sqlx::query_scalar!(
        "select user_id from ticket_release_queuers
        where ticket_kind_id = $1",
        id
    )
    .fetch_all(&mut db.executor())
    .await
    .wrap_err_db()?;

    queuers.shuffle(&mut rng());

    #[allow(
        clippy::cast_sign_loss,
        reason = "it's constrained to be postitive in sql"
    )]
    let (reservations, reservation_queuers) =
        queuers.split_at((ticket_kind.max_tickets as usize).min(queuers.len()));

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

    sqlx::query!(
        "insert into ticket_reservations (user_id, ticket_kind_id, transaction_id, timeout)
        select user_id, $2 as ticket_kind_id, null as transaction_id, (from_now + now()) as timeout
        from unnest($1::text[], $3::interval[]) as t(user_id, from_now)",
        reservations,
        id,
        &timestamps
    )
    .execute(&mut db.executor())
    .await
    .wrap_err_db()?;
    sqlx::query!(
        "insert into ticket_reservation_queuers (user_id, ticket_kind_id, placement)
        select user_id, $2 as ticket_kind_id, placement
        from unnest($1::text[], $3::integer[]) as t(user_id, placement)",
        reservations,
        id,
        &placements
    )
    .execute(&mut db.executor())
    .await
    .wrap_err_db()?;

    Ok(())
}
/// Run this xx:01 (i.e. seconds = 01 & every minute).
/// All DB things this does have to be compatible with being called simultaneously because we might
/// horizontally scale.
///
/// # Errors
///
/// Db. See [`release`].
pub async fn check_all_tickets(db: &PgPool) -> MinilithResult<()> {
    loop {
        let mut txn = db.begin().await.wrap_err_db()?;
        // take one release job
        // this works concurrently too!
        let ticket_kind = sqlx::query!(
            "select id from ticket_kinds
            where
            -- just so if the service crashes, it doesn't release
            -- all the tickets when it comes back up.
            purchasing_available_start > now() - '5 minutes'::interval
            and purchasing_available_start <= now()
            and has_been_released = false
            limit 1
            for update skip locked"
        )
        .fetch_optional(db)
        .await
        .wrap_err_db()?;
        if let Some(row) = ticket_kind {
            release(&mut txn, row.id).await?;
            sqlx::query!(
                "update ticket_kinds set has_been_released = true where ticket_kinds.id = $1",
                row.id
            )
            .execute(&mut txn.executor())
            .await
            .wrap_err_db()?;
            txn.commit().await.wrap_err_db()?;
        } else {
            // there may in certain circumstances be "jobs" left, but they will be taken care of
            // next minute in worst case
            break;
        }
    }

    // Also removes people's queue positions after 20 minutes.
    sqlx::query!(
        "delete from ticket_release_queuers
        where started_queueing < now() - '20 minutes'::interval"
    )
    .execute(db)
    .await
    .wrap_err_db()?;

    // missplaced
    loop {
        let mut txn = db.begin().await.wrap_err_db()?;
        // take one release job
        // this works concurrently too!
        let ticket_kind = sqlx::query!(
            "select user_id, ticket_kind_id from ticket_reservation_queuers
            inner join ticket_kinds kind on (kind.id = ticket_kind_id)
            where kind.has_been_released = true
            limit 1
            for update skip locked"
        )
        .fetch_optional(db)
        .await
        .wrap_err_db()?;
        if let Some(row) = ticket_kind {
            update_misplaced_queuer(&row.user_id, row.ticket_kind_id, &mut txn).await?;
            txn.commit().await.wrap_err_db()?;
        } else {
            // there may in certain circumstances be "jobs" left, but they will be taken care of
            // next minute in worst case
            break;
        }
    }
    // remove_reservation
    while remove_reservation(db).await?.is_continue() {}
    // give_reservations:
    let mut reservations = sqlx::query!(
        "select id as \"ticket_kind_id!\", -- count(user_id),
        (max_tickets - reserved_or_purchased_tickets) as \"available_tickets!\"
        from ticket_reservation_queuers
        inner join ticket_kinds kind on (kind.id = ticket_kind_id)
        where max_tickets > reserved_or_purchased_tickets
        group by id"
    )
    .fetch_all(db)
    .await
    .wrap_err_db()?;
    // shuffle because if multiple runners are trying to do this, make each start at a different
    // node so we don't get as many "for update skip locked" in the start:)
    reservations.shuffle(&mut rng());
    for reservation in reservations {
        let mut txn = db.begin().await.wrap_err_db()?;
        give_reservations(
            reservation.ticket_kind_id,
            reservation.available_tickets,
            &mut txn,
        )
        .await?;
        txn.commit().await.wrap_err_db()?;
    }

    // clear reservation queue when there are no more tickets
    // we use purchased_tickets since they never decrease so the lock on it doesn't matter!
    sqlx::query_scalar!(
        "delete from ticket_reservation_queuers
        where user_id = (
            select user_id
            from ticket_reservation_queuers
            inner join ticket_kinds kind on (kind.id = ticket_kind_id)
            where max_tickets = reserved_or_purchased_tickets
            and (select count(id) from purchased_tickets where ticket_kind_id = kind.id) = max_tickets
            for update skip locked
        )"
    )
    .execute(db)
    .await
    .wrap_err_db()?;

    Ok(())
}
/// Checks for people in the queue once the `ticket_kind` has been released.
/// Assumed that the `ticket_kind` is released.
///
/// # Errors
///
/// DB.
pub async fn update_misplaced_queuer(
    user_id: &str,
    ticket_kind: Uuid,
    db: &mut Transaction<'_>,
) -> MinilithResult<()> {
    sqlx::query!(
        "insert into ticket_reservation_queuers (user_id, ticket_kind_id, placement)
        select $1 as user_id, $2 as ticket_kind_id, queuers.placement + 1 as placement
        from ticket_reservation_queuers queuers
        where queuers.ticket_kind_id = $2
        order by placement desc 
        limit 1",
        user_id,
        ticket_kind
    )
    .execute(&mut db.executor())
    .await
    .wrap_err_db()?;
    sqlx::query!(
        "delete from ticket_release_queuers where user_id = $1",
        user_id
    )
    .execute(&mut db.executor())
    .await
    .wrap_err_db()?;
    Ok(())
}
/// Checks for reservations which are timed out. If any is found, it's removed.
/// Call [`give_reservations`] after calling this.
///
/// # Errors
///
/// Failures from cancelling transactions.
pub async fn remove_reservation(db: &PgPool) -> MinilithResult<ControlFlow<()>> {
    let mut txn = db.begin().await.wrap_err_db()?;
    // only remove reservations which are not in the purchase of buying!
    let removed_reservation = sqlx::query!(
        "select transaction_id, user_id, ticket_kind_id
        from ticket_reservations
        where timeout < now()
        and transaction_id is null
        limit 1
        for update skip locked",
    )
    .fetch_optional(&mut txn.executor())
    .await
    .wrap_err_db()?;
    let do_continue = removed_reservation.is_some();
    if let Some(reservation) = removed_reservation {
        sqlx::query!(
            "update ticket_kinds
            set reserved_or_purchased_tickets = reserved_or_purchased_tickets - 1
            where id = $1",
            reservation.ticket_kind_id,
        )
        .execute(&mut txn.executor())
        .await
        .wrap_err_db()?;
        sqlx::query!(
            "delete from ticket_reservations where user_id = $1 and ticket_kind_id = $2",
            reservation.user_id,
            reservation.ticket_kind_id,
        )
        .execute(&mut txn.executor())
        .await
        .wrap_err_db()?;
    }
    txn.commit().await.wrap_err_db()?;
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
    let removed_reservations = sqlx::query_scalar!(
        "select user_id
        from ticket_reservation_queuers queuers
        where ticket_kind_id = $1
        order by placement asc
        limit $2
        for update skip locked",
        ticket_kind,
        i64::from(fetch_n)
    )
    .fetch_all(&mut db.executor())
    .await
    .wrap_err_db()?;
    if removed_reservations.is_empty() {
        return Ok(());
    }
    let timestamps: Vec<PgInterval> = std::iter::repeat_with(new_timeout_interval)
        .take(removed_reservations.len())
        .collect();
    sqlx::query!(
        "insert into ticket_reservations (user_id, ticket_kind_id, transaction_id, timeout)
        select user_id, $2 as ticket_kind_id, null as transaction_id, (from_now + now()) as timeout
        from unnest($1::text[], $3::interval[]) as t(user_id, from_now)",
        &removed_reservations,
        ticket_kind,
        &timestamps
    )
    .execute(&mut db.executor())
    .await
    .wrap_err_db()?;
    match sqlx::query!(
        "update ticket_kinds
        set reserved_or_purchased_tickets = reserved_or_purchased_tickets + $1
        where id = $2",
        fetch_n,
        ticket_kind
    )
    .execute(&mut db.executor())
    .await
    {
        Err(sqlx::Error::Database(err)) if err.kind() == sqlx::error::ErrorKind::CheckViolation => {
            error!(
                %ticket_kind,
                "tried to reserve too many tickets, this should not happen!",
            );
            return Ok(());
        }
        Err(err) => return Err(err).wrap_err_db(),
        Ok(_) => {}
    }

    sqlx::query!(
        "delete from ticket_reservation_queuers
        where ticket_kind_id = $1
        and user_id = any($2)",
        ticket_kind,
        &removed_reservations
    )
    .execute(&mut db.executor())
    .await
    .wrap_err_db()?;
    Ok(())
}

// ticket buy:
// - [x] place in queue
//   - if queue response is queued, get queue status & display wait &
//     (if reservation queue: refresh every 15 seconds, else refresh after the ticket is released)
//   - if queue response is reserved, go to buy
// - [x] (runtime in minilith releases tickets)
// - [_] go to buy screen
// - user starts transaction
// - transaction backend messages minilith this happened
// - if transaction successful, transfer ticket & move from reserved -> purchased
// - if transaction unsuccessful, return to transact screen, minilith knows this and removes
//   transaction id

/// Ensure that the addons aren't duplicated and that they belong to the
/// specified `ticket_kind`.
async fn validate_addons(
    db: impl PgExecutor<'_>,
    addons: &[Uuid],
    ticket_kind: Uuid,
) -> MinilithResult<()> {
    let mut seen = HashSet::new();
    for &addon in addons {
        if !seen.insert(addon) {
            return Err(MinilithEndpointError::bad_frontend_code(
                format!("addon {addon} is duplicated"),
                "",
            ));
        }
    }

    let count = sqlx::query_scalar!(
        "select count(*) from ticket_addons where id = any($1) and ticket_kind_id = $2",
        addons,
        ticket_kind
    )
    .fetch_one(db)
    .await
    .wrap_err_db()?;

    #[allow(
        clippy::cast_possible_wrap,
        reason = "addons.len() will never exceed i64::MAX"
    )]
    if count != Some(addons.len() as i64) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "number of addons doens't match number of available ones",
            "",
        ));
    }

    Ok(())
}

/// Ensure that the user may purchase a ticket of the specified `ticket_kind`
/// with regard to their group memberships.
///
/// If no allowed groups are configured for the ticket kind, anyone may
/// purchase. Otherwise the user must be a (transitive) member of at least one
/// allowed group — membership in a parent group covers all descendant groups.
///
/// # Errors
///
/// Returns 403 if the user is not allowed to purchase, or an internal error if
/// the database query fails.
async fn ensure_user_may_purchase_ticket(
    db: impl PgExecutor<'_>,
    user: &User,
    ticket_kind: Uuid,
) -> MinilithResult<()> {
    let may_purchase = sqlx::query_scalar!(
        r#"select (
            not exists (
                select 1
                from purchased_tickets
                inner join ticket_kinds kind on kind.id = $1
                inner join ticket_kinds kinds on kinds.activity_id = kind.activity_id
                where
                    purchased_tickets.owner_id = $2
                    and ticket_kind_id = kinds.id
            )
            and
            exists (
                select 1
                from group_memberships
                inner join groups member_group on member_group.id = group_memberships.group_id
                inner join ticket_kind_allowed_groups tk_ag on tk_ag.ticket_kind_id = $1
                inner join groups allowed_group on allowed_group.id = tk_ag.group_id
                    and allowed_group.path @> member_group.path

                where group_memberships.user_id = $2
                and (
                    member_group.limit_membership_visibility = false
                    or tk_ag.group_id = group_memberships.group_id
                )
            )
        ) as "may_purchase!""#,
        ticket_kind,
        user.get_id()
    )
    .fetch_one(db)
    .await
    .wrap_err_db()?;

    if !may_purchase {
        return Err(MinilithEndpointError::bad_user_input(
            "doublette purchase",
            "",
            "not allowed to purchase this ticket kind OR \
            you have already purchased one ticket for this activity",
            "ticket_kind",
        ));
    }

    Ok(())
}
