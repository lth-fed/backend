use std::fmt::Display;
use std::ops::Deref;
use std::sync::Arc;

use fed_auth_verifier::callbacks::{TransactionInfo, TransactionState};
use minilith_errors::{
    MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult,
};
use poem::http::HeaderMap;
use poem_openapi::param::Path;
use poem_openapi::payload::{Binary, Json, Response};
use poem_openapi::{Enum, Object, OpenApi};
use serde::Serialize;
use sqlx::postgres::types::PgMoney;
use time::OffsetDateTime;
use tracing::error;
use uuid::Uuid;

use crate::context::CancelTransactionData;
use crate::{ApiAuth, Context, Provider, callback, receipt, swish};

#[derive(Default, Debug, Enum, Clone, Copy, Serialize)]
pub enum Currency {
    /// Swedish crowns.
    #[oai(rename = "SEK")]
    #[default]
    Sek,
}
impl Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sek => f.write_str("SEK"),
        }
    }
}
#[derive(Debug, Object, Clone, Serialize)]
pub struct Ware {
    /// When buying several of one item, append e.g. `x3` to the name and increase `amount`.
    /// This is flexible for e.g. sales when buying more than 1.
    pub name: String,
    /// The total amount (inclusive tax) for this ware. In ören.
    pub amount: u32,
    /// The tax rate. Must be `> 1` (e.g. `1.25` for common moms in Sweden).
    pub tax: f64,
    /// The currency in which this transactions is made.
    pub currency: Currency,
}
#[derive(Debug, Object, Clone)]
struct CreatePaymentRequest {
    /// This tells us which account to put the money into!
    client_id: String,
    /// When this payment request will be cancelled.
    /// Will try to cancel within 30s.
    timeout: OffsetDateTime,
    /// The list of items to be bought.
    wares: Vec<Ware>,
}
#[derive(Debug, Object, Clone)]
struct CreatePaymentResponseSwish {
    /// See <https://developer.swish.nu/api/payment-request/v2#create-payment-request>.
    payment_request_token: String,
    transaction_id: Uuid,
}

#[derive(Debug, Enum, Clone)]
#[oai(rename_all = "lowercase")]
enum ReceiptLanguage {
    En,
    Sv,
}
#[derive(Debug, Object, Clone)]
struct ReceiptRequest {
    language: ReceiptLanguage,
    customer_name: String,
}

#[derive(Debug)]
pub struct Route {
    pub context: Arc<Context>,
}
impl Deref for Route {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

fn check_client_id(auth: &ApiAuth, txn_client_id: &str) -> MinilithResult<()> {
    if txn_client_id != auth.client_id {
        return Err(MinilithEndpointError::bad_user_input(
            "client_id doesn't match on receipt",
            "",
            "you are not allowed to view this receipt",
            "client_id",
        ));
    }
    Ok(())
}

#[OpenApi]
impl Route {
    #[oai(path = "/:id", method = "get")]
    async fn state(
        &self,
        auth: ApiAuth,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<TransactionInfo>> {
        let Some(transaction) = sqlx::query!(
            "select payment_reference is not null as \"paid!\", client_id,
            refund_reference is not null as \"refunded!\"
            from transactions
            where id = $1",
            id
        )
        .fetch_optional(&self.db)
        .await
        .wrap_err_db()?
        else {
            return Ok(Json(TransactionInfo {
                state: TransactionState::Cancelled,
            }));
        };
        check_client_id(&auth, &transaction.client_id)?;
        let state = if transaction.refunded {
            TransactionState::Refunded
        } else if transaction.paid {
            TransactionState::Paid
        } else {
            TransactionState::Pending
        };
        Ok(Json(TransactionInfo { state }))
    }
    #[oai(path = "/:id/cancel", method = "post")]
    async fn cancel(&self, auth: ApiAuth, Path(id): Path<Uuid>) -> MinilithResult<()> {
        let row = sqlx::query!(
            "select client_id, callback_url_v1, provider as \"provider!: crate::Provider\"
            from transactions where id = $1",
            id
        )
        .fetch_optional(&self.db)
        .await
        .wrap_err_db()?
        .wrap_err_not_found()?;
        check_client_id(&auth, &row.client_id)?;
        self.cancel_transaction(&CancelTransactionData {
            id,
            callback_url_v1: row.callback_url_v1,
            provider: row.provider,
        })
        .await?;
        Ok(())
    }
    /// You WILL get info on the callback (unless it's unreachable) about either it getting
    /// cancelled or paid. We will do our best within our control.
    #[oai(path = "/swish", method = "post")]
    async fn swish_payment(
        &self,
        auth: ApiAuth,
        body: Json<CreatePaymentRequest>,
    ) -> MinilithResult<Json<CreatePaymentResponseSwish>> {
        check_client_id(&auth, &body.client_id)?;

        let amount = body.wares.iter().fold(0, |acc, ware| acc + ware.amount);
        if amount < 100 {
            return Err(MinilithEndpointError::bad_user_input(
                "low amount",
                "",
                "amount is less than 1SEK!",
                "amount",
            ));
        }

        let client = sqlx::query!(
            "select * from client_ids where client_id = $1",
            auth.client_id
        )
        .fetch_one(&self.db)
        .await
        .wrap_err_db()?;

        let uuid = Uuid::new_v4();
        let cb_ident = Uuid::new_v4();
        let mut amount = amount.to_string();
        amount.insert(amount.len() - 2, '.');
        let mut message = body
            .wares
            .iter()
            .fold(String::new(), |acc, ware| (acc + &ware.name) + ", ");
        message.pop();
        message.pop();
        message.retain(|char| "!?(),.-:; åäöÅÄÖ".contains(char) || char.is_ascii_alphanumeric());
        message.truncate(50);
        let swish_body = swish::CreatePaymentRequest {
            payee_alias: client.swish_number,
            amount,
            currency: "SEK".to_owned(),
            callback_url: "https://transactions.teknologappen.se/v0/swish-callback".to_owned(),
            message,
            callback_identifier: swish::uuid_to_string(cb_ident),
        };
        let resp = self
            .swish_client
            .put(swish::payment_request_url(uuid))
            .json(&swish_body)
            .send()
            .await
            .map_err(|err| {
                MinilithEndpointError::internal_error(format!("swish api error: {err:?}"))
            })?;
        if !resp.status().is_success() {
            let body = resp.text().await;
            error!(error = ?body, "got non 2xx response from swish API");
            return Err(MinilithEndpointError::internal_error(""));
        }
        let prt = resp
            .headers()
            .get("PaymentRequestToken")
            .and_then(|header| header.to_str().ok())
            .ok_or_else(|| {
                MinilithEndpointError::internal_error("swish gave us no PaymentRequestToken")
            })?;

        let mut txn = self.db.begin().await.wrap_err_db()?;
        sqlx::query!(
            "
            insert into transactions (id, client_id, callback_url_v1,
                total_transaction_fee, callback_identifier)
            values ($1, $2, $3, '1.50'::money, $4)
            ",
            uuid,
            auth.client_id,
            auth.callback_url_v1,
            cb_ident
        )
        .execute(&mut txn.executor())
        .await
        .wrap_err_db()?;

        // wares
        #[allow(
            clippy::cast_possible_wrap,
            clippy::cast_possible_truncation,
            reason = "bruh"
        )]
        let idxes = body
            .wares
            .iter()
            .enumerate()
            .map(|(idx, _)| idx as i32)
            .collect::<Vec<_>>();
        let transaction_ids = std::iter::repeat_n(uuid, body.wares.len()).collect::<Vec<_>>();
        let names = body
            .wares
            .iter()
            .map(|ware| ware.name.clone())
            .collect::<Vec<_>>();
        let amounts = body
            .wares
            .iter()
            .map(|ware| PgMoney(i64::from(ware.amount)))
            .collect::<Vec<_>>();
        let currencies = body
            .wares
            .iter()
            .map(|ware| ware.currency.to_string())
            .collect::<Vec<_>>();
        let taxes = body.wares.iter().map(|ware| ware.tax).collect::<Vec<_>>();
        sqlx::query!(
            "insert into transaction_wares
            (idx, transaction_id, name, amount, currency, tax)
            select idx, transaction_id, name, amount, currency, tax 
            from unnest($1::integer[], $2::uuid[], $3::text[], $4::money[], $5::text[],
                $6::double precision[])
            as t(idx, transaction_id, name, amount, currency, tax)",
            &idxes,
            &transaction_ids,
            &names,
            &amounts,
            &currencies,
            &taxes
        )
        .execute(&mut txn.executor())
        .await
        .wrap_err_db()?;
        txn.commit().await.wrap_err_db()?;

        Ok(Json(CreatePaymentResponseSwish {
            transaction_id: uuid,
            payment_request_token: prt.to_owned(),
        }))
    }
    /// This endpoint is called by Swish's backend.
    #[oai(path = "/swish-callback", method = "post")]
    async fn swish_callback(
        &self,
        body: Json<swish::Callback>,
        headers: &HeaderMap,
    ) -> MinilithResult<()> {
        let callback_identifier = headers
            .get("callbackIdentifier")
            .and_then(|header| header.to_str().ok())
            .and_then(|header| Uuid::parse_str(header).ok())
            .ok_or_else(|| {
                MinilithEndpointError::unauthorized("callbackIdentifier not valid", "")
            })?;

        callback::handle_callback_to_us(self, body.0, Some(callback_identifier)).await?;

        Ok(())
    }
    /// Must be made from the same `client_id` as the transaction.
    #[oai(path = "/:id/refund", method = "post")]
    async fn refund(&self, auth: ApiAuth, Path(id): Path<Uuid>) -> MinilithResult<Json<String>> {
        let transaction = sqlx::query!(
            "select client_id, payment_reference, refund_reference
            from transactions where id = $1",
            id,
        )
        .fetch_optional(&self.db)
        .await
        .wrap_err_db()?
        .wrap_err_not_found()?;

        check_client_id(&auth, &transaction.client_id)?;
        if transaction.payment_reference.is_none() {
            return Err(MinilithEndpointError::bad_user_input(
                "refund when not paid",
                "",
                "refund is not available until after the purchase is complete",
                "<paid>",
            ));
        }

        Ok(Json(String::new()))
    }
    /// Must be made from the same `client_id` as the transaction.
    #[oai(path = "/:id/receipt", method = "post")]
    async fn receipt(
        &self,
        auth: ApiAuth,
        Path(id): Path<Uuid>,
        body: Json<ReceiptRequest>,
    ) -> MinilithResult<Response<Binary<Vec<u8>>>> {
        let transaction = sqlx::query!(
            "select id, payment_reference, refund_reference,
            client_id, created, provider as \"provider!: Provider\"
            from transactions where id = $1",
            id
        )
        .fetch_optional(&self.db)
        .await
        .wrap_err_db()?
        .wrap_err_not_found()?;

        check_client_id(&auth, &transaction.client_id)?;
        let Some(payment_reference) = transaction.payment_reference else {
            return Err(MinilithEndpointError::bad_user_input(
                "receipt when not paid",
                "",
                "receipt is not available until after the purchase is complete",
                "<paid>",
            ));
        };

        let client_id = sqlx::query!(
            "select * from client_ids where client_id = $1",
            transaction.client_id
        )
        .fetch_one(&self.db)
        .await
        .wrap_err_db()?;
        #[allow(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "pls not that much money, besides we insert u32"
        )]
        let wares = sqlx::query!(
            "select name, amount, currency, tax
            from transaction_wares
            where transaction_id = $1
            order by idx asc",
            id
        )
        .map(|row| Ware {
            name: row.name,
            amount: row.amount.0 as u32,
            tax: row.tax,
            currency: Currency::default(),
        })
        .fetch_all(&self.db)
        .await
        .wrap_err_db()?;
        let data = receipt::Data {
            transaction_id: id.to_string(),
            purchase_date: transaction
                .created
                .date()
                .format(&time::format_description::well_known::Iso8601::DATE)
                .unwrap_or_default(),
            provider: transaction.provider,
            payment_reference,
            refund_refrence: transaction.refund_reference,
            wares,
            customer_name: body.customer_name.clone(),
            merchant_id: transaction.client_id,
            merchant_name: client_id.name,
            merchant_org_id: client_id.organization_number,
            merchant_email: client_id.email,
            merchant_address: client_id.address,
        };
        let doc = receipt::compile(&self.typst_world, &data);
        Ok(Response::new(Binary(doc)).header("content-type", "application/octet-stream"))
    }
}
