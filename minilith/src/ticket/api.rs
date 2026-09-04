use fed_auth_verifier::User;
use poem_openapi::{
    OpenApi,
    param::Path,
    payload::{Binary, Json, Response},
};
use uuid::Uuid;

use super::transfer::TransferRequest;
use super::validation::{ValidateActivity, ValidateRequest, ValidateResponse};
use super::{
    Router, catalog,
    models::{
        BuyTicketRequest, BuyTicketResponse, Kind, PurchaseStatus, PurchasedTicket, QueueRequest,
        QueueResponse,
    },
    purchase, queue, transfer, validation,
};
use crate::MinilithResult;

#[OpenApi(prefix_path = "/tickets")]
impl Router {
    /// # Errors
    ///
    /// AUTH, DB
    #[oai(path = "/", method = "get")]
    #[allow(clippy::too_many_lines, reason = "it's linear")]
    async fn my_tickets(&self, user: User) -> MinilithResult<Json<Vec<PurchasedTicket>>> {
        catalog::my_tickets(self, user).await.map(Json)
    }

    #[oai(path = "/:id/receipt", method = "get")]
    async fn receipt(
        &self,
        auth: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Response<Binary<poem::Body>>> {
        catalog::receipt(self, auth, id).await
    }

    #[oai(path = "/ticket-kind/:id", method = "get")]
    async fn get_ticket_kind(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Kind>> {
        catalog::get_ticket_kind(self, user.get_id(), id)
            .await
            .map(Json)
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
    #[allow(
        clippy::too_many_lines,
        reason = "keeps the three purchase-flow transitions and their lock order together"
    )]
    async fn queue(
        &self,
        user: User,
        req: Json<QueueRequest>,
    ) -> MinilithResult<Json<PurchaseStatus>> {
        queue::queue(self, user, req.0).await.map(Json)
    }
    /// Get the status of the queue. If 404 & user has started transacting, this means the purchase
    /// went through!
    ///
    /// # Errors
    ///
    /// - 404 not found when the user is not queued (neither reservation queue nor release queue)
    #[oai(path = "/queue", method = "get")]
    async fn queue_status(&self, user: User) -> MinilithResult<Json<QueueResponse>> {
        queue::status(self, user).await.map(Json)
    }
    /// Cancel the reservation or drop out of queue if the user is no longer interested in buying
    /// it (e.g. realize they are broke).
    ///
    /// Cancelled / dropped if this returns 200.
    ///
    /// # Errors
    ///
    /// - 404 not found when the user doesn't have a reservation
    /// - 403 if another operation with the transaction flow is happening
    #[oai(path = "/queue", method = "delete")]
    async fn drop_transaction_flow(&self, user: User) -> MinilithResult<()> {
        queue::drop_transaction_flow(self, user).await
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
        body: Json<BuyTicketRequest>,
    ) -> MinilithResult<Json<BuyTicketResponse>> {
        purchase::begin(self, user, body.0).await.map(Json)
    }

    /// You must own the ticket. The recipient must belong to one of the kind's transfer groups or
    /// a descendant, and this must be called between `Kind.allow_transfer_ticket_start` and
    /// `Kind.allow_transfer_ticket_stop`.
    /// Check these values by fetching the data of the Kind using `/v0/tickets/ticket-kind/<uuid>`
    #[oai(path = "/transfer", method = "post")]
    async fn transfer(&self, auth: User, body: Json<TransferRequest>) -> MinilithResult<()> {
        transfer::transfer(self, auth, body.0).await
    }

    #[oai(path = "/validate", method = "get")]
    async fn validatable_activities(
        &self,
        auth: User,
    ) -> MinilithResult<Json<Vec<ValidateActivity>>> {
        validation::validatable_activities(self, auth)
            .await
            .map(Json)
    }

    #[oai(path = "/validate", method = "post")]
    #[allow(
        clippy::too_many_lines,
        reason = "it's linear and just has a bunch of sql queries"
    )]
    async fn validate(
        &self,
        auth: User,
        body: Json<ValidateRequest>,
    ) -> MinilithResult<Json<ValidateResponse>> {
        validation::validate(self, auth, body.0).await.map(Json)
    }

    /// `/v0/tickets/callback`
    ///
    /// # Errors
    ///
    /// DB errors.
    #[oai(path = "/callback", method = "post")]
    #[allow(
        clippy::too_many_lines,
        reason = "keeps callback states and the cancellation lock order visible in one match"
    )]
    pub async fn callback(
        &self,
        events: fed_auth_verifier::callbacks::TransactionsCallbackDataV1,
    ) -> MinilithResult<()> {
        purchase::callback(self, events).await
    }
}
