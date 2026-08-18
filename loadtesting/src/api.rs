use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::auth::Session;
use crate::error::{Result, ResultContext as _, error};

#[derive(Clone, Debug)]
pub struct ApiClient {
    http: Client,
    api_url: Url,
    token_url: Url,
}

impl ApiClient {
    pub fn new(http: Client, api_url: Url, auth_url: &Url) -> Result<Self> {
        Ok(Self {
            http,
            api_url,
            token_url: auth_url
                .join("oidc/v1/token")
                .context("construct refresh-token URL")?,
        })
    }

    pub async fn ticket_kind(
        &self,
        session: &mut Session,
        ticket_kind: Uuid,
    ) -> Result<TicketKind> {
        let url = self
            .api_url
            .join(&format!("tickets/ticket-kind/{ticket_kind}"))
            .context("construct ticket-kind URL")?;
        let token = session.access_token(&self.http, &self.token_url).await?;
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context(format!("fetch ticket kind for {}", session.user_id))?;
        decode_success(response, "ticket-kind request").await
    }

    pub async fn enter_queue(
        &self,
        session: &mut Session,
        ticket_kind: Uuid,
    ) -> Result<QueueEntry> {
        let url = self
            .api_url
            .join("tickets/queue")
            .context("construct queue URL")?;
        let token = session.access_token(&self.http, &self.token_url).await?;
        let response = self
            .http
            .put(url)
            .bearer_auth(token)
            .json(&QueueRequest { ticket_kind })
            .send()
            .await
            .context(format!("enter queue for {}", session.user_id))?;
        if response.status().is_success() {
            return response
                .json()
                .await
                .map(QueueEntry::Status)
                .context("decode queue entry");
        }
        if response.status() == StatusCode::BAD_REQUEST {
            let api_error: ApiError = response.json().await.context("decode queue-entry error")?;
            if api_error.field.as_deref() == Some("ticket_kind_id") {
                return Ok(QueueEntry::SoldOut);
            }
            return Err(error(format!(
                "queue entry returned 400 Bad Request: {}",
                api_error.message
            )));
        }
        Err(response_error(response, "queue entry").await)
    }

    pub async fn queue_status(&self, session: &mut Session) -> Result<QueuePoll> {
        let url = self
            .api_url
            .join("tickets/queue")
            .context("construct queue-status URL")?;
        let token = session.access_token(&self.http, &self.token_url).await?;
        let response = match self.http.get(url).bearer_auth(token).send().await {
            Ok(response) => response,
            Err(source) => return Ok(QueuePoll::Retry(source.to_string())),
        };
        match response.status() {
            StatusCode::NOT_FOUND => Ok(QueuePoll::Missing),
            status if status.is_server_error() => {
                let body = limited_body(response).await;
                Ok(QueuePoll::Retry(format!("{status}: {body}")))
            }
            status if status.is_success() => response
                .json()
                .await
                .map(QueuePoll::Status)
                .context("decode queue status"),
            _ => Err(response_error(response, "queue status").await),
        }
    }

    pub async fn buy_free(
        &self,
        session: &mut Session,
        ticket_kind: Uuid,
    ) -> Result<BuyFreeOutcome> {
        let url = self
            .api_url
            .join("tickets/reservation/buy")
            .context("construct reservation purchase URL")?;
        let token = session.access_token(&self.http, &self.token_url).await?;
        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&BuyTicketRequest {
                ticket_kind,
                provider: "free",
                addons: Vec::new(),
            })
            .send()
            .await
            .context(format!("buy free ticket for {}", session.user_id))?;
        if response.status().is_success() {
            return Ok(BuyFreeOutcome::Started);
        }
        if response.status() == StatusCode::FORBIDDEN {
            return Ok(BuyFreeOutcome::PurchaseFlowBusy);
        }
        Err(response_error(response, "free purchase").await)
    }

    pub async fn owns_ticket(&self, session: &mut Session, ticket_kind: Uuid) -> Result<bool> {
        let url = self
            .api_url
            .join("tickets")
            .context("construct owned-tickets URL")?;
        let token = session.access_token(&self.http, &self.token_url).await?;
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context(format!("list tickets for {}", session.user_id))?;
        let tickets: Vec<OwnedTicket> = decode_success(response, "owned-tickets request").await?;
        Ok(tickets
            .iter()
            .any(|ticket| ticket.ticket_kind_id == ticket_kind))
    }
}

#[derive(Debug, Deserialize)]
pub struct TicketKind {
    #[serde(rename = "ticket_kind_id")]
    pub id: Uuid,
    pub price: i64,
    pub purchasing_available_start: String,
    pub available_addons: Vec<AvailableAddon>,
}

impl TicketKind {
    pub fn release_at(&self) -> Result<OffsetDateTime> {
        OffsetDateTime::parse(&self.purchasing_available_start, &Rfc3339)
            .context("parse ticket release time")
    }
}

#[derive(Debug, Deserialize)]
pub struct AvailableAddon {
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum PurchaseStatus {
    ReleaseQueued,
    ReservationQueued,
    Reserved,
    Buying,
    Purchased,
}

#[derive(Clone, Copy, Debug)]
pub enum QueueEntry {
    Status(PurchaseStatus),
    SoldOut,
}

#[derive(Clone, Copy, Debug)]
pub enum BuyFreeOutcome {
    Started,
    PurchaseFlowBusy,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: String,
    field: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QueueResponse {
    pub ticket_kind: Uuid,
    pub placement: Option<i32>,
    pub timeout: Option<String>,
    pub start_transaction_before: Option<String>,
}

#[derive(Debug)]
pub enum QueuePoll {
    Status(QueueResponse),
    Missing,
    Retry(String),
}

#[derive(Serialize)]
struct QueueRequest {
    ticket_kind: Uuid,
}

#[derive(Serialize)]
struct BuyTicketRequest<'a> {
    ticket_kind: Uuid,
    provider: &'a str,
    addons: Vec<()>,
}

#[derive(Debug, Deserialize)]
struct OwnedTicket {
    ticket_kind_id: Uuid,
}

async fn decode_success<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    context: &str,
) -> Result<T> {
    if !response.status().is_success() {
        return Err(response_error(response, context).await);
    }
    response.json().await.context(format!("decode {context}"))
}

async fn response_error(response: reqwest::Response, context: &str) -> crate::error::Error {
    let status = response.status();
    let body = limited_body(response).await;
    error(format!("{context} returned {status}: {body}"))
}

async fn limited_body(response: reqwest::Response) -> String {
    response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(1_000)
        .collect()
}
