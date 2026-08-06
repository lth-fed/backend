use std::collections::HashMap;
use std::ops::{ControlFlow, Deref};

use bin_common::{PgPool, Transaction};
use fed_auth_verifier::User;
use fed_auth_verifier::callbacks::TransactionState;
use minilith_errors::{
    AlertLevel, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, alert,
};
use poem_openapi::Enum;
use poem_openapi::param::Path;
use poem_openapi::payload::{Binary, Response};
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
    MinilithEndpointError, MinilithResult, transactions,
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

impl Router {
    /// Loads a ticket kind without checking activity access. Callers must
    /// authorize the request before returning the value to a client.
    #[allow(
        clippy::too_many_lines,
        reason = "keeps the existing ticket-kind query and mapping in one reusable loader"
    )]
    pub(crate) async fn load_ticket_kind_unchecked(&self, id: Uuid) -> MinilithResult<Kind> {
        let mut ticket_kind = sqlx::query!(
            "select
                name as \"name!: DIS\", activity_id, price,
                purchasing_available_start, purchasing_available_stop,
                max_tickets, min_tickets, reserved_or_purchased_tickets,
                allow_transfer_ticket_start, allow_transfer_ticket_stop,
                allow_transfer_ticket_bypass_allowed_groups,
                has_been_purchased,
                has_been_released
            from ticket_kinds where id = $1",
            id
        )
        .map(|row| Kind {
            inner: TicketBase {
                ticket_kind_id: id,
                ticket_kind_name: row.name.0,
                activity_id: row.activity_id,
            },
            price: row.price.0,
            purchasing_available_start: row.purchasing_available_start,
            purchasing_available_stop: row.purchasing_available_stop,
            max_tickets: row.max_tickets,
            min_tickets: row.min_tickets,
            reserved_or_purchased_tickets: row.reserved_or_purchased_tickets,
            allow_transfer_ticket_start: row.allow_transfer_ticket_start,
            allow_transfer_ticket_stop: row.allow_transfer_ticket_stop,
            allow_transfer_ticket_bypass_allowed_groups: row
                .allow_transfer_ticket_bypass_allowed_groups,
            has_been_purchased: row.has_been_purchased,
            has_been_released: row.has_been_released,
            allowed_group_ids: Vec::new(),
            available_addons: Vec::new(),
        })
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;

        ticket_kind.allowed_group_ids = sqlx::query_scalar!(
            r#"select group_id from ticket_kind_allowed_groups
            where ticket_kind_id = $1 order by group_id"#,
            id,
        )
        .fetch_all(&self.db)
        .await?;

        let options: HashMap<Uuid, Vec<AddonOption>> = sqlx::query!(
            "select ticket_addon_options.id, ticket_addon_id, ticket_addon_options.idx,
            ticket_addon_options.name as \"name: DIS\", price,
            -- wait wtf this Vec<i64> syntax actually works??
            bookkeeping_prices as \"bkp: Vec<i64>\", bookkeeping_price_categories
            from ticket_addon_options
            inner join ticket_addons on (ticket_addons.id = ticket_addon_options.ticket_addon_id)
            where ticket_kind_id = $1
            order by ticket_addon_options.idx",
            id
        )
        .map(|row| {
            (
                row.ticket_addon_id,
                AddonOption {
                    id: row.id,
                    idx: row.idx,
                    name: row.name.0,
                    price: row.price.0,
                    bookkeeping_prices: row.bkp,
                    bookkeeping_price_categories: row.bookkeeping_price_categories,
                },
            )
        })
        .fetch_all(&self.context.db)
        .await?
        .into_iter()
        .fold(HashMap::new(), |mut map, (addon_id, option)| {
            map.entry(addon_id).or_default().push(option);
            map
        });
        ticket_kind.available_addons = sqlx::query!(
            "select id, name as \"name: DIS\",
            multiple_alternatives, has_text_field, required
            from ticket_addons
            where ticket_kind_id = $1
            order by ticket_addons.idx",
            id
        )
        .map(|row| AvailableAddon {
            inner: Addon {
                id: row.id,
                name: row.name.0,
                multiple_alternatives: row.multiple_alternatives,
                has_text_field: row.has_text_field,
                required: row.required,
            },
            options: options.get(&row.id).cloned().unwrap_or_default(),
        })
        .fetch_all(&self.context.db)
        .await?;

        Ok(ticket_kind)
    }
}

#[derive(Debug, Clone, Copy, Enum, PartialEq, Eq)]
#[oai(rename_all = "lowercase")]
pub enum PurchaseProvider {
    Swish,
    Stripe,
    Free,
}
#[derive(Debug, Clone, Object)]
pub struct BoughtAddon {
    id: Uuid,
    selected_text: Option<String>,
    selected_options: Option<Vec<i32>>,
}
#[derive(Debug, Clone, Object)]
pub struct BuyTicketRequest {
    ticket_kind: Uuid,
    /// Doesn't matter for free tickets.
    provider: PurchaseProvider,
    addons: Vec<BoughtAddon>,
    /// Required for stripe.
    stripe_success_url: Option<String>,
}
#[derive(Debug, Clone, Object)]
pub struct BuyTicketResponse {
    /// Not null when using [`PurchaseProvider::Swish`].
    payment_request_token: Option<String>,
    /// Not null when using [`PurchaseProvider::Stripe`].
    /// Open this in a new browser context.
    /// Close that context when [`BuyTicketRequest::stripe_success_url`] is reached.
    stripe_url: Option<String>,
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
    start_transaction_before: Option<OffsetDateTime>,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct PurchaseStatusRequest {
    activity: Uuid,
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

#[derive(Object, Debug)]
pub struct Addon {
    pub id: Uuid,
    pub name: IS,
    pub multiple_alternatives: bool,
    pub has_text_field: bool,
    pub required: bool,
}
#[derive(Object, Debug)]
pub struct PurchasedAddon {
    #[oai(flatten)]
    pub inner: Addon,
    pub selected_options: Vec<i32>,
    pub selected_text: String,
}
#[derive(Object, Clone, Debug)]
pub struct AddonOption {
    pub id: Uuid,
    pub idx: i32,
    pub name: IS,
    pub price: i64,
    // for admins mostly
    pub bookkeeping_prices: Vec<i64>,
    pub bookkeeping_price_categories: Vec<String>,
}
#[derive(Object, Debug)]
pub struct AvailableAddon {
    #[oai(flatten)]
    pub inner: Addon,
    pub options: Vec<AddonOption>,
}

#[allow(clippy::module_name_repetitions, reason = "Base is a shit name")]
#[derive(Object, Debug)]
pub struct TicketBase {
    #[allow(clippy::struct_field_names, reason = "reasonable name")]
    pub ticket_kind_id: Uuid,
    #[allow(clippy::struct_field_names, reason = "reasonable name")]
    pub ticket_kind_name: IS,
    pub activity_id: Uuid,
}
#[derive(Object, Debug)]
struct PurchasedTicket {
    #[oai(flatten)]
    inner: TicketBase,
    id: Uuid,
    activity_location: PoemLocation,
    activity_title: IS,
    creator_id: Uuid,
    creator_path: String,
    creator_name: IS,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    purchased_addons: Vec<PurchasedAddon>,
    /// False if we have transferred it.
    owned_by_me: bool,
}
#[derive(Object, Debug)]
pub struct Kind {
    #[oai(flatten)]
    pub inner: TicketBase,
    pub price: i64,
    pub purchasing_available_start: OffsetDateTime,
    pub purchasing_available_stop: OffsetDateTime,
    pub max_tickets: i32,
    pub min_tickets: i32,
    pub reserved_or_purchased_tickets: i32,
    pub allow_transfer_ticket_start: OffsetDateTime,
    pub allow_transfer_ticket_stop: OffsetDateTime,
    pub allow_transfer_ticket_bypass_allowed_groups: bool,
    pub has_been_purchased: bool,
    pub has_been_released: bool,
    pub allowed_group_ids: Vec<Uuid>,
    pub available_addons: Vec<AvailableAddon>,
}

impl Kind {
    pub(crate) fn activity_id(&self) -> Uuid {
        self.inner.activity_id
    }

    pub(crate) fn reserved_or_purchased_tickets(&self) -> i32 {
        self.reserved_or_purchased_tickets
    }

    pub(crate) fn has_been_purchased(&self) -> bool {
        self.has_been_purchased
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "compares the corresponding API fields"
    )]
    pub(crate) fn immutable_fields_match(
        &self,
        activity_id: Uuid,
        price: i64,
        allowed_group_ids: &[Uuid],
        addons: &[AvailableAddon],
    ) -> bool {
        self.inner.activity_id == activity_id
            && self.price == price
            && self.allowed_group_ids == allowed_group_ids
            && self.available_addons.len() == addons.len()
            && self.available_addons.iter().zip(addons).all(|(old, new)| {
                old.inner.id == new.inner.id
                    && old.inner.name == new.inner.name
                    && old.inner.multiple_alternatives == new.inner.multiple_alternatives
                    && old.inner.has_text_field == new.inner.has_text_field
                    && old.inner.required == new.inner.required
                    && old.options.len() == new.options.len()
                    && old
                        .options
                        .iter()
                        .zip(&new.options)
                        .all(|(old_option, new_option)| {
                            old_option.id == new_option.id
                                && old_option.name == new_option.name
                                && old_option.price == new_option.price
                        })
            })
    }
}

#[derive(Object)]
struct TransferRequest {
    purchased_ticket_id: Uuid,
    to_user: String,
}

#[derive(Object)]
struct ValidateActivity {
    id: Uuid,
    title: IS,
    description: IS,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
}

/// The frontend has to encode / decode the QR with both these datapoints, maybe through
/// `<id>.<time>` or JSON.
#[derive(Object)]
struct ValidateRequest {
    purchased_ticket_id: Uuid,
    created_at: OffsetDateTime,
}
#[derive(Object)]
struct Validation {
    at: OffsetDateTime,
}
#[derive(Object)]
struct ValidateResponse {
    verified: bool,
    owner_id: Option<String>,
    owner_name: Option<String>,
    previous_verifications: Vec<Validation>,
}
impl ValidateResponse {
    pub fn not_valid() -> Self {
        Self {
            verified: false,
            owner_id: None,
            owner_name: None,
            previous_verifications: vec![],
        }
    }
}

#[OpenApi(prefix_path = "/tickets")]
impl Router {
    /// # Errors
    ///
    /// AUTH, DB
    #[oai(path = "/", method = "get")]
    async fn my_tickets(&self, user: User) -> MinilithResult<Json<Vec<PurchasedTicket>>> {
        let id = user.get_id();

        let mut addons: HashMap<Uuid, Vec<PurchasedAddon>> = sqlx::query!(
            r#"select
                purchased_ticket_addons.ticket_id as "ticket_id",
                ticket_addons.id as "addon_id",
                ticket_addons.name as "addon_name: DIS",
                ticket_addons.multiple_alternatives as "multiple_alternatives",
                ticket_addons.has_text_field as "has_text_field",
                ticket_addons.required as "required",
                purchased_ticket_addons.selected_options as "selected_options",
                purchased_ticket_addons.selected_text as "selected_text"
            from purchased_tickets
            inner join purchased_ticket_addons on
                purchased_ticket_addons.ticket_id = purchased_tickets.id
            inner join ticket_addons on
                ticket_addons.id = purchased_ticket_addons.addon_id
            where purchased_tickets.owner_id = $1 or purchased_tickets.purchaser_id = $1
            order by ticket_addons.idx
            "#,
            id
        )
        .map(|row| {
            (
                row.ticket_id,
                PurchasedAddon {
                    inner: Addon {
                        id: row.addon_id,
                        name: row.addon_name.0,
                        multiple_alternatives: row.multiple_alternatives,
                        has_text_field: row.has_text_field,
                        required: row.required,
                    },
                    selected_options: row.selected_options,
                    selected_text: row.selected_text,
                },
            )
        })
        .fetch_all(&self.context.db)
        .await?
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
                activities.time_end as "time_end",
                (owner_id = purchaser_id) as "owned_by_me!"
            from purchased_tickets
            inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
            inner join activities on activities.id = ticket_kinds.activity_id
            inner join groups creator on creator.id = activities.creator_id
            where purchased_tickets.owner_id = $1 or purchased_tickets.purchaser_id = $1
            "#,
            id
        )
        .map(|ticket| PurchasedTicket {
            inner: TicketBase {
                ticket_kind_id: ticket.ticket_kind_id,
                ticket_kind_name: ticket.ticket_kind_name.0,
                activity_id: ticket.activity_id,
            },
            id: ticket.id,
            activity_location: ticket.location.into(),
            activity_title: ticket.activity_title.0,
            creator_id: ticket.creator_id,
            creator_path: ticket.creator_path.to_string(),
            creator_name: ticket.creator_name.0,
            time_start: ticket.time_start,
            time_end: ticket.time_end,
            purchased_addons: addons.remove(&ticket.id).unwrap_or_default(),
            owned_by_me: ticket.owned_by_me,
        })
        .fetch_all(&self.context.db)
        .await?;

        Ok(Json(tickets))
    }

    #[oai(path = "/:id/receipt", method = "get")]
    async fn receipt(
        &self,
        auth: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Response<Binary<poem::Body>>> {
        let Some(transaction_id) = sqlx::query_scalar!(
            "select transaction_id
            from purchased_tickets
            where id = $1
            and purchaser_id = $2",
            id,
            auth.get_id(),
        )
        .fetch_optional(&self.db)
        .await?
        else {
            return Err(MinilithEndpointError::not_found());
        };

        let user = sqlx::query!(
            "select name as \"name: DIS\", language
            from users where id = $1",
            auth.get_id()
        )
        .fetch_one(&self.db)
        .await?;

        let lang = self
            .decrypt_string(user.language)
            .wrap_err_encryption("user.language")?;
        let receipt_lang = match lang.get(..2) {
            Some("sv") => transactions::Language::Swedish,
            _ => transactions::Language::English,
        };
        let name = user.name.resolve_intl(&lang, "<name>");

        let data = transactions::ReceiptRequest {
            language: receipt_lang,
            customer_name: name.to_owned(),
        };
        let resp = self
            .transactions_post(format!("/v0/{transaction_id}/receipt"))
            .json(&data)
            .send()
            .await
            .wrap_err_internal("receipt failed to fetch")?
            .error_for_status()
            .wrap_err_internal("receipt status code error")?
            .bytes()
            .await
            .wrap_err_internal("receipt read body")?;
        Ok(Response::new(Binary(resp.into())).header("content-type", "application/octet-stream"))
    }

    #[oai(path = "/ticket-kind/:id", method = "get")]
    async fn get_ticket_kind(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Kind>> {
        let ticket_kind = self.load_ticket_kind_unchecked(id).await?;

        self.test_activity_access(user.get_id(), &ticket_kind.activity_id())
            .await?;

        Ok(Json(ticket_kind))
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
        ensure_user_may_purchase_ticket(&self.db, user.get_id(), req.ticket_kind).await?;
        let has_reservation = sqlx::query_scalar!(
            "select exists (
                select 1 from ticket_reservations where user_id = $1
            ) as \"exists!\"",
            user.get_id()
        )
        .fetch_one(&self.db)
        .await?;
        if has_reservation {
            return Err(MinilithEndpointError::bad_frontend_code(
                "user already has reservation, cancel it before queuing again",
                "",
            ));
        }
        let row = sqlx::query!(
            "select has_been_released, purchasing_available_stop from ticket_kinds where id = $1",
            req.ticket_kind
        )
        .fetch_one(&self.db)
        .await?;

        if row.purchasing_available_stop < OffsetDateTime::now_utc() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "ticket not available for purchase anymore",
                "",
            ));
        }

        if row.has_been_released {
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
            .await?;

            if row.reserved_or_purchased_tickets < row.max_tickets && row.count == 0 {
                let mut txn = self.db.begin().await?;
                if reserve_ticket_capacity(&mut txn, req.ticket_kind, 1).await? == 1 {
                    // give reservation
                    sqlx::query!(
                        "insert into ticket_reservations
                        (user_id, ticket_kind_id, transaction_id, timeout)
                        values ($1, $2, null, now() + $3)",
                        user.get_id(),
                        req.ticket_kind,
                        new_timeout_interval()
                    )
                    .execute(&mut txn.executor())
                    .await?;
                    if txn.commit().await.is_ok() {
                        return Ok(Json(PurchaseStatus::Reserved));
                    }
                }
                // if the txn fails, it's because we've tried to reserve too many, stand in queue
                // instead:
            }

            sqlx::query!(
                "insert into ticket_reservation_queuers (user_id, ticket_kind_id, placement) \
                select $1, $2,
                    -- take last placement, add one
                    coalesce(
                        (select placement
                        from ticket_reservation_queuers
                        order by placement desc limit 1),
                    0) + 1
                
                where not exists (
                    select 1 from purchased_tickets where ticket_kind_id = $2 and owner_id = $1
                )
                on conflict (user_id) do update
                set ticket_kind_id = excluded.ticket_kind_id, placement = excluded.placement",
                user.get_id(),
                req.ticket_kind
            )
            .execute(&self.db)
            .await?;
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
            .execute(&self.db)
            .await?;
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
        .await?;
        if rows.rows_affected() == 0 {
            // not MinilithEndpointError::not_found() since that gives an error
            Err(MinilithEndpointError::NotFound(Json(
                minilith_errors::MinilithError::new("no reservation"),
            )))
        } else {
            Ok(())
        }
    }
    /// Get the status of the queue. If 404 & user has started transacting, this means the purchase
    /// went through!
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
        .await?;
        if let Some(row) = reservation {
            return Ok(Json(QueueResponse {
                ticket_kind: row.ticket_kind_id,
                placement: Some(0),
                timeout: Some(row.timeout),
                start_transaction_before: Some(row.timeout - 1 * time::Duration::MINUTE),
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
        .await?;
        if let Some(row) = reservation_queue {
            return Ok(Json(QueueResponse {
                ticket_kind: row.ticket_kind_id,
                placement: Some((row.placement - row.reserved_or_purchased_tickets).max(0)),
                timeout: None,
                start_transaction_before: None,
            }));
        }
        let queuer = sqlx::query_scalar!(
            "select ticket_kind_id from ticket_release_queuers where user_id = $1",
            user.get_id()
        )
        .fetch_optional(&self.db)
        .await?;
        if let Some(id) = queuer {
            return Ok(Json(QueueResponse {
                ticket_kind: id,
                placement: None,
                timeout: None,
                start_transaction_before: None,
            }));
        }
        Err(MinilithEndpointError::not_found())
    }
    /// Cancel the reservation if the user is no longer interested in buying it (e.g. realize they
    /// are broke).
    ///
    /// Cancelled if this returns 200.
    ///
    /// # Errors
    ///
    /// - 404 not found when the user doesn't have a reservation
    #[oai(path = "/reservation", method = "delete")]
    async fn drop_reservation(&self, user: User) -> MinilithResult<()> {
        let mut txn = self.db.begin().await?;
        let Some(row) = sqlx::query!(
            "delete from ticket_reservations where user_id = $1
            returning ticket_kind_id, transaction_id",
            user.get_id(),
        )
        .fetch_optional(&mut txn.executor())
        .await?
        else {
            return Err(MinilithEndpointError::NotFound(Json(
                minilith_errors::MinilithError::new("no reservation"),
            )));
        };
        // try to cancel transaction instead
        if let Some(id) = row.transaction_id {
            let resp = self
                .transactions_post(format!("/v0/{id}/cancel"))
                .send()
                .await
                .wrap_err_internal("failed to cancel transaction due to connection issues")?;
            if !resp.status().is_success() {
                return Err(MinilithEndpointError::internal_error(
                    "l1: transaction cancel failed!",
                    resp.status(),
                ));
            }
            // transaction is cancelled
        }
        sqlx::query!(
            "update ticket_kinds
            set reserved_or_purchased_tickets = reserved_or_purchased_tickets - 1
            where id = $1",
            row.ticket_kind_id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "update ticket_reservations
            set transaction_id = null
            where user_id = $1",
            user.get_id(),
        )
        .execute(&mut txn.executor())
        .await?;
        give_reservations(row.ticket_kind_id, 1, &mut txn).await?;
        txn.commit().await?;
        Ok(())
    }
    /// Try to lock in this reservation by purchasing the ticket.
    /// If a transaction is already underway, it's cancelled.
    ///
    /// To see when you've gotten the ticket, poll `GET /queue`. When it's gone (404), you should
    /// have a ticket (or the timeout expired). To check which, list the owned tickets (`GET /`), if
    /// one from this activity is there it's purchased.
    ///
    /// # Errors
    ///
    /// - addons invalid (they should match the valid addons you got getting the details of this
    ///   `ticket_kind`)
    /// - `ticket_kind` doesn't match current reservation
    /// - could not cancel transaction (500)
    /// - user already owns a ticket from this event
    #[oai(path = "/reservation/buy", method = "post")]
    #[allow(
        clippy::too_many_lines,
        reason = "It's linear and well-documented. \
        It's easier to read in its whole than if it was in multiple functions."
    )]
    async fn begin_purchase(
        &self,
        user: User,
        Json(mut body): Json<BuyTicketRequest>,
    ) -> MinilithResult<Json<BuyTicketResponse>> {
        if body.provider == PurchaseProvider::Stripe && body.stripe_success_url.is_none() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "stripe_success_url has to be non-null when provider is stripe!",
                "",
            ));
        }
        // this is here so nobody else tries to mess with our reservation while we are assigning it
        // a transaction_ìd
        let mut txn = self.db.begin().await?;
        let reservation = sqlx::query!(
            "select ticket_reservations.id, ticket_kind_id, timeout, transaction_id,
                kind.name as \"ticket_kind_name!: DIS\",
                activities.title as \"activity_title!: DIS\",
                kind.price
            from ticket_reservations
            inner join ticket_kinds kind on (kind.id = ticket_kind_id)
            inner join activities on (activities.id = kind.activity_id)
            where user_id = $1
            for update",
            user.get_id()
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .wrap_err_not_found()?;
        if reservation.ticket_kind_id != body.ticket_kind {
            return Err(MinilithEndpointError::bad_frontend_code(
                "ticket_kind you requested to buy is not the same as the one you have reserved",
                "",
            ));
        }
        if let Some(txn_id) = reservation.transaction_id {
            let resp = self
                .transactions_post(format!("/v0/{txn_id}/cancel"))
                .send()
                .await
                .wrap_err_internal("failed to cancel transaction")?;
            if let Err(error) = resp.error_for_status_ref() {
                return Err(MinilithEndpointError::internal_error(
                    "l1: failed to cancel transaction due to status code",
                    error,
                ));
            }
            sqlx::query!(
                "update ticket_reservations set transaction_id = null where user_id = $1",
                user.get_id()
            )
            .execute(&mut txn.executor())
            .await?;
        }

        // addons for a ticket_kind are immutable so we don't do it through a transaction
        let chosen_options = validate_addons(&self.db, &mut body.addons, body.ticket_kind).await?;
        ensure_user_may_purchase_ticket(&self.db, user.get_id(), body.ticket_kind).await?;

        // ========
        // remove old addons
        // ========
        sqlx::query!(
            "delete from ticket_reservation_addons where ticket_id = $1",
            reservation.id
        )
        .execute(&mut txn.executor())
        .await?;

        // we can't insert `unnest($1::integer[][])` for selected_options because postgres is weird
        // and represents 2D-arrays as a 1D array it'd get ugly
        for addon in &body.addons {
            sqlx::query!(
                "insert into ticket_reservation_addons
                (addon_id, ticket_id, selected_options, selected_text)
                values ($1, $2, $3, $4)",
                addon.id,
                reservation.id,
                addon.selected_options.as_deref().unwrap_or(&[]),
                addon.selected_text.as_deref().unwrap_or(""),
            )
            .execute(&mut txn.executor())
            .await?;
        }

        // ========
        // prepare Ware:s for transaction API
        // ========
        let lang = sqlx::query_scalar!("select language from users where id = $1", user.get_id())
            .fetch_one(&self.db)
            .await?;
        let lang = self
            .decrypt_string(lang)
            .wrap_err_encryption("failed to decrypt user language")?;

        let ticket_kind_name = reservation
            .ticket_kind_name
            .resolve_intl(&lang, "<ticket kind>");
        let activity_title = reservation.activity_title.resolve_intl(&lang, "<activity>");

        // we don't need to include ticket_kind because the ticket_addon_id is also a UUID so it
        // will never be duplicate!
        let available_addons = sqlx::query!(
            "select id, name as \"name!: DIS\", idx
            from ticket_addons
            where id = any($1)",
            &body.addons.iter().map(|addon| addon.id).collect::<Vec<_>>()
        )
        .fetch_all(&self.db)
        .await?;

        let mut transaction_wares = vec![transactions::Ware {
            name: format!("{activity_title} - {ticket_kind_name}"),
            amount: reservation.price.0,
            tax: 1.25,
            currency: transactions::Currency::Sek,
        }];
        let get_addon_idx = |id: Uuid| {
            available_addons
                .iter()
                .find(|addon| addon.id == id)
                .map_or(0, |addon| addon.idx)
        };
        // these got shuffled by `validate_addons`.
        body.addons
            .sort_unstable_by_key(|addon| get_addon_idx(addon.id));
        for addon in &body.addons {
            let info = available_addons
                .iter()
                .find(|available| available.id == addon.id)
                .wrap_err_internal(
                    "we previously guaranteed (I though) that all options \
                    were in the DB and loaded. They were not.",
                )?;
            let addon_name = info.name.resolve_intl(&lang, "<addon>");
            // closure move bullshit, apparently we can't just move some variables...
            let lang = lang.clone();
            let options = chosen_options
                .iter()
                .filter(|opt| opt.ticket_addon_id == addon.id)
                .map(move |opt| {
                    let option_name = opt.name.resolve_intl(&lang, "<option>");
                    transactions::Ware {
                        name: format!("    {addon_name} - {option_name}"),
                        amount: opt.price,
                        tax: 1.25,
                        currency: transactions::Currency::Sek,
                    }
                });
            transaction_wares.extend(options);
        }
        // ========
        // Get UUID
        // ========
        let transaction_id: Uuid = self
            .transactions_post("/v0/init")
            .send()
            .await
            .wrap_err_internal("init transport failed")?
            .error_for_status()
            .wrap_err_internal("init status")?
            .json()
            .await
            .wrap_err_internal("l1: init bad type")?;
        sqlx::query!(
            "update ticket_reservations set transaction_id = $1
            where id = $2",
            transaction_id,
            reservation.id
        )
        .execute(&mut txn.executor())
        .await?;

        txn.commit().await?;

        // ========
        // Send transaction API request
        // ========
        let timeout = reservation
            .timeout
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .wrap_err_internal("failed to format value which we got & serialized before")?;
        let payment_req = transactions::CreatePaymentRequest {
            id: transaction_id,
            customer_id: Some(user.get_id().to_owned()),
            timeout,
            wares: transaction_wares,
            stripe_success_url: body.stripe_success_url,
        };
        let total_amount = payment_req
            .wares
            .iter()
            .fold(0, |acc, ware| acc + ware.amount);
        if body.provider == PurchaseProvider::Free && total_amount != 0 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "cannot pay for non-free ticket with free provider",
                "",
            ));
        }
        let provider = if total_amount == 0 {
            PurchaseProvider::Free
        } else {
            body.provider
        };
        let url = match provider {
            PurchaseProvider::Free => "/v0/free",
            PurchaseProvider::Swish => "/v0/swish",
            PurchaseProvider::Stripe => "/v0/stripe",
        };
        let resp = match self.transactions_post(url).json(&payment_req).send().await {
            Ok(resp) => resp,
            Err(err) => {
                return Err(MinilithEndpointError::internal_error(
                    "failed to buy ticket due to connection issues",
                    err,
                ));
            }
        };
        if !resp.status().is_success() {
            return Err(MinilithEndpointError::internal_error(
                "failed to start transaction due to us being bad",
                "",
            ));
        }
        // ========
        // Handle transaction API response
        // ========
        let response = match provider {
            PurchaseProvider::Free => BuyTicketResponse {
                payment_request_token: None,
                stripe_url: None,
            },
            // these have to be separate match arms because the response type is different
            PurchaseProvider::Swish => {
                let body = resp
                    .json::<transactions::CreatePaymentResponseSwish>()
                    .await
                    .wrap_err_internal(
                        "failed to start transaction due to us being bad in parsing",
                    )?;
                BuyTicketResponse {
                    payment_request_token: Some(body.payment_request_token),
                    stripe_url: None,
                }
            }
            PurchaseProvider::Stripe => {
                let body = resp
                    .json::<transactions::CreatePaymentResponseStripe>()
                    .await
                    .wrap_err_internal(
                        "failed to start transaction due to us being bad in parsing",
                    )?;
                BuyTicketResponse {
                    payment_request_token: None,
                    stripe_url: Some(body.redirect_url),
                }
            }
        };

        Ok(Json(response))
    }

    /// You must own the ticket, and can if `Kind.allow_transfer_ticket_bypass_allowed_groups ==
    /// false` only transfer it to other users who could buy this ticket. This must also be called
    /// between `Kind.allow_transfer_ticket_start` and `Kind.allow_transfer_ticket_stop`.
    /// Check these values by fetching the data of the Kind using `/v0/tickets/ticket-kind/<uuid>`
    #[oai(path = "/transfer", method = "post")]
    async fn transfer(&self, auth: User, body: Json<TransferRequest>) -> MinilithResult<()> {
        let mut txn = self.db.begin().await?;
        let row = sqlx::query!(
            "select allow_transfer_ticket_bypass_allowed_groups, ticket_kind_id,
            allow_transfer_ticket_start, allow_transfer_ticket_stop
            from purchased_tickets
            inner join ticket_kinds kind on kind.id = purchased_tickets.ticket_kind_id
            where purchased_tickets.id = $1 and owner_id = $2
            for update of purchased_tickets",
            body.purchased_ticket_id,
            auth.get_id()
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .wrap_err_bad_frontend("you don't own this ticket")?;
        let other_owns = sqlx::query_scalar!(
            "select exists (
                select 1 from purchased_tickets where owner_id = $1 and ticket_kind_id = $2
            ) or exists (
                select 1 from ticket_reservations where user_id = $1 and ticket_kind_id = $2
            ) or exists (
                select 1 from ticket_reservation_queuers where user_id = $1 and ticket_kind_id = $2
            ) or exists (
                select 1 from ticket_release_queuers where user_id = $1 and ticket_kind_id = $2
            ) as \"owns!\"",
            body.to_user,
            row.ticket_kind_id
        )
        .fetch_one(&mut txn.executor())
        .await?;
        if other_owns {
            return Err(MinilithEndpointError::bad_user_input(
                "transfer to other user who already owns",
                "",
                "the receiving user already owns or is buying a ticket to this event",
                "to_user",
            ));
        }
        let now = OffsetDateTime::now_utc();
        if row.allow_transfer_ticket_stop <= now || row.allow_transfer_ticket_start >= now {
            return Err(MinilithEndpointError::bad_frontend_code(
                "cannot transfer ticket at this time",
                "",
            ));
        }
        if !row.allow_transfer_ticket_bypass_allowed_groups {
            ensure_user_may_purchase_ticket(&mut txn.executor(), &body.to_user, row.ticket_kind_id)
                .await?;
        }
        sqlx::query!(
            "update purchased_tickets set owner_id = $2 where id = $1",
            body.purchased_ticket_id,
            body.to_user
        )
        .execute(&mut txn.executor())
        .await?;

        txn.commit().await?;

        Ok(())
    }

    #[oai(path = "/validate", method = "get")]
    async fn validate_activities(&self, auth: User) -> MinilithResult<Json<Vec<ValidateActivity>>> {
        sqlx::query!(
            "select title as \"title!: DIS\", description as \"description!: DIS\",
            a.id, url, time_start, time_end
            from activity_verifiers
            inner join activities a on a.id = activity_verifiers.activity_id
            inner join images on images.id = a.image_id
            where user_id = $1
            and a.time_end > now() - '24 hours'::interval
            and a.time_start < now() + '7 days'::interval",
            auth.get_id()
        )
        .map(|row| ValidateActivity {
            id: row.id,
            title: row.title.0,
            description: row.description.0,
            time_start: row.time_start,
            time_end: row.time_end,
            image_url: row.url,
        })
        .fetch_all(&self.db)
        .await
        .map_err(Into::into)
        .map(Json)
    }

    #[oai(path = "/validate", method = "post")]
    async fn validate(
        &self,
        auth: User,
        body: Json<ValidateRequest>,
    ) -> MinilithResult<Json<ValidateResponse>> {
        let now = OffsetDateTime::now_utc();
        let min5 = time::Duration::MINUTE * 5;
        if body.created_at < now.saturating_sub(min5) || body.created_at > now.saturating_add(min5)
        {
            return Ok(Json(ValidateResponse::not_valid()));
        }
        let Some(row) = sqlx::query!(
            "select owner_id, users.name
            from purchased_tickets 
            inner join ticket_kinds kind on kind.id = purchased_tickets.ticket_kind_id
            inner join activity_verifiers on activity_verifiers.activity_id = kind.activity_id
            inner join users on users.id = owner_id
            where purchased_tickets.id = $1 
                and activity_verifiers.user_id = $2",
            body.purchased_ticket_id,
            auth.get_id()
        )
        .fetch_optional(&self.db)
        .await?
        else {
            return Ok(Json(ValidateResponse::not_valid()));
        };
        sqlx::query!(
            "insert into purchased_ticket_validations (id, purchased_ticket_id)
            values ($1, $2)",
            Uuid::new_v4(),
            body.purchased_ticket_id
        )
        .execute(&self.db)
        .await?;
        let previous_verifications = sqlx::query!(
            "select timestamp from purchased_ticket_validations where purchased_ticket_id = $1",
            body.purchased_ticket_id
        )
        .map(|row| Validation { at: row.timestamp })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(ValidateResponse {
            verified: true,
            owner_id: Some(row.owner_id),
            owner_name: Some(
                self.decrypt_string(row.name)
                    .wrap_err_encryption("validate name")?,
            ),
            previous_verifications,
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
                    let mut txn = self.db.begin().await?;
                    if pay_for_reservation(&mut txn, data.transaction_id)
                        .await?
                        .is_some()
                    {
                        txn.commit().await?;
                    } else {
                        txn.rollback().await?;
                    }
                }
                TransactionState::Refunded => {
                    let affected = sqlx::query!(
                        "update purchased_tickets set owner_id = 'refunded:'
                    where transaction_id = $1",
                        data.transaction_id
                    )
                    .execute(&self.db)
                    .await?;
                    if affected.rows_affected() != 1 {
                        alert(AlertLevel::L1, "1 row not affected when purchase refunded!");
                        error!(transaction_id=%data.transaction_id,
                            "1 row not affected when purchase refunded!"
                        );
                    }
                }
                TransactionState::Cancelled => {
                    let mut txn = self.db.begin().await?;
                    let Some(row) = sqlx::query!(
                        "update ticket_reservations
                        set transaction_id = null
                        where transaction_id = $1
                        returning
                            id,
                            ticket_kind_id,
                            timeout < now() as \"has_timed_out!\"",
                        data.transaction_id,
                    )
                    .fetch_optional(&mut txn.executor())
                    .await?
                    else {
                        error!(
                            transaction_id = %data.transaction_id,
                            "transaction which we do not track is cancelled"
                        );
                        alert(
                            AlertLevel::L2,
                            "transaction which we do not track is cancelled",
                        );
                        continue;
                    };
                    if row.has_timed_out {
                        sqlx::query!("delete from ticket_reservations where id = $1", row.id)
                            .execute(&mut txn.executor())
                            .await?;
                        sqlx::query!(
                            r#"update ticket_kinds
                            set reserved_or_purchased_tickets =
                                reserved_or_purchased_tickets - 1
                            where id = $1"#,
                            row.ticket_kind_id,
                        )
                        .execute(&mut txn.executor())
                        .await?;
                        give_reservations(row.ticket_kind_id, 1, &mut txn).await?;
                    }
                    txn.commit().await?;
                }
            }
        }
        Ok(())
    }
}
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
    .await?;

    let mut queuers = sqlx::query_scalar!(
        "select user_id from ticket_release_queuers
        where ticket_kind_id = $1",
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
    let granted = reserve_ticket_capacity(db, id, requested).await?;
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
    // release tickets
    loop {
        let mut txn = db.begin().await?;
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
        .await?;
        if let Some(row) = ticket_kind {
            release(&mut txn, row.id).await?;
            sqlx::query!(
                "update ticket_kinds set has_been_released = true where ticket_kinds.id = $1",
                row.id
            )
            .execute(&mut txn.executor())
            .await?;
            txn.commit().await?;
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
    .await?;

    // missplaced
    loop {
        let mut txn = db.begin().await?;
        // take one release job
        // this works concurrently too!
        let ticket_kind = sqlx::query!(
            "select user_id, ticket_kind_id from ticket_release_queuers
            inner join ticket_kinds kind on (kind.id = ticket_kind_id)
            where kind.has_been_released = true
            limit 1
            for update skip locked"
        )
        .fetch_optional(db)
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
    .await?;
    // shuffle because if multiple runners are trying to do this, make each start at a different
    // node so we don't get as many "for update skip locked" in the start:)
    reservations.shuffle(&mut rng());
    for reservation in reservations {
        let mut txn = db.begin().await?;
        give_reservations(
            reservation.ticket_kind_id,
            reservation.available_tickets,
            &mut txn,
        )
        .await?;
        txn.commit().await?;
    }

    remove_queuers_when_sold_out(db).await?;

    Ok(())
}
/// Clear reservation queue when there are no more tickets.
/// We use `purchased_tickets` since they never decrease so the lock on it doesn't matter!
async fn remove_queuers_when_sold_out(db: impl PgExecutor<'_>) -> MinilithResult<()> {
    sqlx::query!(
        r#"delete from ticket_reservation_queuers queuer
        using ticket_kinds kind
        where kind.id = queuer.ticket_kind_id
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
        )"#
    )
    .execute(db)
    .await?;
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
        select $1 as user_id, $2 as ticket_kind_id, coalesce((
            select placement
            from ticket_reservation_queuers reserv
            where reserv.ticket_kind_id = $2
            order by placement desc 
            limit 1
        ), kind.reserved_or_purchased_tickets) + 1 as placement
        from ticket_release_queuers queuers
        inner join ticket_kinds kind on kind.id = $2
        where queuers.ticket_kind_id = $2
        limit 1",
        user_id,
        ticket_kind
    )
    .execute(&mut db.executor())
    .await?;
    sqlx::query!(
        "delete from ticket_release_queuers where user_id = $1",
        user_id
    )
    .execute(&mut db.executor())
    .await?;
    Ok(())
}
/// Checks for reservations which are timed out. If any is found, it's removed.
/// Call [`give_reservations`] after calling this.
///
/// # Errors
///
/// Failures from cancelling transactions.
pub async fn remove_reservation(db: &PgPool) -> MinilithResult<ControlFlow<()>> {
    let mut txn = db.begin().await?;
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
    .await?;
    let do_continue = removed_reservation.is_some();
    if let Some(reservation) = removed_reservation {
        sqlx::query!(
            "update ticket_kinds
            set reserved_or_purchased_tickets = reserved_or_purchased_tickets - 1
            where id = $1",
            reservation.ticket_kind_id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "delete from ticket_reservations where user_id = $1 and ticket_kind_id = $2",
            reservation.user_id,
            reservation.ticket_kind_id,
        )
        .execute(&mut txn.executor())
        .await?;
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
    let mut removed_reservations = sqlx::query_scalar!(
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
    .await?;
    if removed_reservations.is_empty() {
        return Ok(());
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "we'll never get this high"
    )]
    let granted =
        reserve_ticket_capacity(db, ticket_kind, removed_reservations.len() as i32).await?;
    #[allow(clippy::cast_sign_loss, reason = "removed will always be positive")]
    removed_reservations.truncate(granted as usize);
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
    .await?;
    sqlx::query!(
        "delete from ticket_reservation_queuers
        where ticket_kind_id = $1
        and user_id = any($2)",
        ticket_kind,
        &removed_reservations
    )
    .execute(&mut db.executor())
    .await?;
    Ok(())
}

/// # Returns
///
/// Returns the id of ticket. None if shit went down internally. Rollback in that case.
async fn pay_for_reservation(
    txn: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        "insert into purchased_tickets
        (id, purchaser_id, owner_id, ticket_kind_id, transaction_id)
        select id, user_id as purchaser_id, user_id as owner_id,
        ticket_kind_id, transaction_id
        from ticket_reservations reserv
            where transaction_id = $1
        returning id",
        transaction_id
    )
    .fetch_optional(&mut txn.executor())
    .await?;
    // has this already been marked as purchased?
    let Some(id) = id else {
        let exists_purchased_ticket = sqlx::query_scalar!(
            "select exists (
                select 1 from purchased_tickets where transaction_id = $1
            ) as \"exists!\"",
            transaction_id
        )
        .fetch_one(&mut txn.executor())
        .await?;
        if !exists_purchased_ticket {
            // ono somebody paid for a non-existing ticket!!
            error!(%transaction_id,
                "tried to pay for an unaccounted-for ticket"
            );
            alert(AlertLevel::L1, "tried to pay for an unknown ticket");
        }
        // otherwise, we're golden, this is just a second "person has paid" callback.
        return Ok(None);
    };
    sqlx::query!(
        r#"update ticket_kinds
        set has_been_purchased = true
        where id = (
            select ticket_kind_id from purchased_tickets where id = $1
        )"#,
        id,
    )
    .execute(&mut txn.executor())
    .await?;
    // move addons:
    sqlx::query!(
        "insert into purchased_ticket_addons
        (addon_id, ticket_id, selected_options, selected_text)
        select addon_id, ticket_id, selected_options, selected_text
        from ticket_reservation_addons
            where ticket_id = $1",
        id,
    )
    .execute(&mut txn.executor())
    .await?;
    sqlx::query!(
        "delete from ticket_reservation_addons
            where ticket_id = $1",
        id,
    )
    .execute(&mut txn.executor())
    .await?;
    // end move addons

    let affected = sqlx::query!(
        "delete from ticket_reservations where transaction_id = $1",
        transaction_id
    )
    .execute(&mut txn.executor())
    .await?;
    if affected.rows_affected() != 1 {
        error!(%transaction_id,
            "1 row not affected when purchase complete!"
        );
        alert(AlertLevel::L1, "1 row not affected when purchase complete!");
        return Ok(None);
    }
    Ok(Some(id))
}

struct ReturnedAddonOption {
    ticket_addon_id: Uuid,
    name: DIS,
    price: i64,
}

/// Ensure that the addons aren't duplicated and that they belong to the
/// specified `ticket_kind`.
#[allow(
    clippy::too_many_lines,
    reason = "it's quite linear and does a single function"
)]
async fn validate_addons(
    db: &PgPool,
    addons: &mut [BoughtAddon],
    ticket_kind: Uuid,
) -> MinilithResult<Vec<ReturnedAddonOption>> {
    addons.sort_unstable_by_key(|addon| addon.id);
    addons
        .iter()
        .zip(addons.iter().skip(1))
        .try_for_each(|(one_of_them, the_one_after)| {
            if one_of_them.id == the_one_after.id {
                Err(MinilithEndpointError::bad_frontend_code(
                    format!("addon {} is duplicated", one_of_them.id),
                    "",
                ))
            } else {
                Ok(())
            }
        })?;

    let addon_ids = addons.iter().map(|addon| addon.id).collect::<Vec<_>>();
    let addon_data = sqlx::query!(
        "select has_text_field, required, multiple_alternatives,
        name as \"name!: DIS\"
        from unnest($1::uuid[]) as t(id) 
        inner join ticket_addons on ticket_addons.id = t.id
            and ticket_kind_id = $2
        order by t.id",
        &addon_ids,
        ticket_kind
    )
    .fetch_all(db)
    .await?;
    if addon_data.len() != addons.len() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "not all addons exist!",
            "",
        ));
    }

    // pairwise we need to verify these
    // they have the same order
    let selected_options_ids = addons
        .iter()
        .flat_map(|addon| addon.selected_options.iter().flatten().map(|_| addon.id))
        .collect::<Vec<_>>();
    let selected_options_idxes = addons
        .iter()
        .flat_map(|addon| addon.selected_options.iter().flatten())
        .copied()
        .collect::<Vec<_>>();

    let valid_indices = sqlx::query_as!(
        ReturnedAddonOption,
        "with input as (
            select ticket_addon_id, idx from
            unnest($1::uuid[], $2::integer[]) as t(ticket_addon_id, idx)
        )
        select opts.ticket_addon_id, name as \"name!: DIS\", price as \"price!: i64\"
        from input
        inner join ticket_addon_options opts
            on (opts.ticket_addon_id = input.ticket_addon_id and opts.idx = input.idx)",
        &selected_options_ids,
        &selected_options_idxes
    )
    .fetch_all(db)
    .await?;

    if selected_options_ids.len() != valid_indices.len() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "selected_options contains some indices which were not valid",
            "",
        ));
    }

    for (addon, row) in addons.iter_mut().zip(addon_data.iter()) {
        if !row.has_text_field && addon.selected_text.is_some() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "addon has text even though this is not allowed",
                "",
            ));
        }
        let n_options = usize::from(
            row.has_text_field
                && addon
                    .selected_text
                    .as_ref()
                    .is_some_and(|text| !text.trim().is_empty()),
        ) + addon.selected_options.as_ref().map_or(0, Vec::len);

        if row.required && n_options == 0 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "required addon missing option",
                "",
            ));
        }
        if !row.multiple_alternatives && n_options > 1 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "too many selected options! Only 1 is permitted",
                "",
            ));
        }

        if let Some(options) = &mut addon.selected_options {
            let before_len = options.len();
            options.sort_unstable();
            options.dedup();
            if options.len() != before_len {
                return Err(MinilithEndpointError::bad_frontend_code(
                    format!(
                        "duplicate option for addon {}",
                        row.name.resolve_intl("en", "")
                    ),
                    "",
                ));
            }
        }
    }

    Ok(valid_indices)
}

/// Ensure that the user may purchase a ticket of the specified `ticket_kind`
/// with regard to their group memberships.
///
/// If no allowed groups are configured for the ticket kind, no one may
/// purchase. Otherwise the user must be a (transitive) member of at least one
/// allowed group — membership in a parent group covers all descendant groups.
///
/// # Errors
///
/// Returns 403 if the user is not allowed to purchase, or an internal error if
/// the database query fails.
async fn ensure_user_may_purchase_ticket(
    db: impl PgExecutor<'_>,
    user_id: &str,
    ticket_kind: Uuid,
) -> MinilithResult<()> {
    let may_purchase = sqlx::query_scalar!(
        r#"select (
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
        user_id,
    )
    .fetch_one(db)
    .await?;

    if !may_purchase {
        return Err(MinilithEndpointError::bad_user_input(
            "purchase",
            "",
            "not allowed to purchase this ticket kind OR \
            you have already purchased one ticket for this activity",
            "ticket_kind",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(fixtures("ticket_capacity"))]
    async fn reservation_capacity_is_shared_by_ticket_kinds(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        let first = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
        let second = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();

        let mut txn = db.begin().await.unwrap();
        assert_eq!(
            reserve_ticket_capacity(&mut txn, first, 2).await.unwrap(),
            2
        );
        assert_eq!(
            reserve_ticket_capacity(&mut txn, second, 2).await.unwrap(),
            1
        );
        assert_eq!(
            reserve_ticket_capacity(&mut txn, first, 1).await.unwrap(),
            0
        );
        txn.commit().await.unwrap();

        let total = sqlx::query_scalar!(
            r#"select sum(reserved_or_purchased_tickets)::int
            from ticket_kinds where activity_id =
                '00000000-0000-0000-0000-000000000003'"#
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(total, Some(3));
    }

    #[sqlx::test(fixtures("ticket_capacity"))]
    async fn concurrent_ticket_kinds_cannot_exceed_activity_capacity(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        let first = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
        let second = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();

        let reserve = |ticket_kind| {
            let db = db.clone();
            async move {
                let mut txn = db.begin().await.unwrap();
                let granted = reserve_ticket_capacity(&mut txn, ticket_kind, 2)
                    .await
                    .unwrap();
                txn.commit().await.unwrap();
                granted
            }
        };
        let (first_granted, second_granted) = tokio::join!(reserve(first), reserve(second));
        assert_eq!(first_granted + second_granted, 3);

        let total = sqlx::query_scalar!(
            r#"select sum(reserved_or_purchased_tickets)::int
            from ticket_kinds where activity_id =
                '00000000-0000-0000-0000-000000000003'"#
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(total, Some(3));
    }
}
