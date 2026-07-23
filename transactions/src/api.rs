use std::fmt::Display;
use std::ops::Deref;
use std::sync::Arc;

use fed_auth_verifier::callbacks::{TransactionCallbackInfo, TransactionInfo, TransactionState};
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
use uuid::Uuid;

use crate::context::CancelTransactionData;
use crate::{ApiAuth, CallbackEvent, Context, Provider, callback, receipt, swish};

#[derive(Default, Debug, Enum, Clone, Copy, Serialize)]
#[oai(rename_all = "UPPERCASE")]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    /// Swedish crowns.
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
    #[oai(validator(minimum(value = "0", exclusive = false)))]
    pub amount: i64,
    /// The tax rate. Must be `> 1` (e.g. `1.25` for common moms in Sweden).
    pub tax: f64,
    /// The currency in which this transactions is made.
    pub currency: Currency,
}
#[derive(Debug, Object, Clone)]
struct CreatePaymentRequest {
    /// When this payment request will be cancelled.
    /// Will try to cancel within 30s.
    timeout: OffsetDateTime,
    /// The list of items to be bought.
    wares: Vec<Ware>,
}
impl CreatePaymentRequest {
    // remove when the SEK restriction is lifted
    fn total_amount(&self) -> i64 {
        self.wares.iter().map(|ware| ware.amount).sum()
    }
}
#[derive(Debug, Object, Clone)]
struct CreatePaymentResponseFree {
    transaction_id: Uuid,
}
#[derive(Debug, Object, Clone)]
struct CreatePaymentResponseSwish {
    /// See <https://developer.swish.nu/api/payment-request/v2#create-payment-request>.
    payment_request_token: String,
    transaction_id: Uuid,
}

#[derive(Debug, Object, Clone)]
struct ReceiptRequest {
    language: receipt::Language,
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
        .await?
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
        .await?
        .wrap_err_not_found()?;
        check_client_id(&auth, &row.client_id)?;
        self.cancel_transaction(&CancelTransactionData {
            id,
            callback_url_v1: row.callback_url_v1,
            client_id: row.client_id,
            provider: row.provider,
        })
        .await?;
        Ok(())
    }
    /// You WILL NOT get info on the callback, the transaction will be marked paid instantly.
    ///
    /// Keep in mind to complete your transaction before calling this, else we might call your
    /// callback prematurely.
    ///
    /// # Errors
    ///
    /// - the amount was not 0
    #[oai(path = "/free", method = "post")]
    async fn free_payment(
        &self,
        auth: ApiAuth,
        body: Json<CreatePaymentRequest>,
    ) -> MinilithResult<Json<CreatePaymentResponseFree>> {
        let amount = body.total_amount();
        if amount != 0 {
            return Err(MinilithEndpointError::bad_user_input(
                "amount",
                "",
                "this transaction is not for 0SEK!",
                "amount",
            ));
        }

        let mut txn = self.db.begin().await?;
        let transaction_id = Uuid::new_v4();
        sqlx::query!(
            "insert into transactions (id, client_id, callback_url_v1,
                total_transaction_fee, callback_identifier, payment_reference)
            values ($1, $2, $3, '0.00'::money, $4, 'free')
            ",
            transaction_id,
            auth.client_id,
            auth.callback_url_v1,
            Uuid::nil()
        )
        .execute(&mut txn.executor())
        .await?;

        insert_wares(&mut txn.executor(), transaction_id, &body.wares).await?;
        txn.commit().await?;

        callback::send_callbacks(
            &self.client,
            &self.signing_key,
            [CallbackEvent {
                callback_url_v1: auth.callback_url_v1.clone(),
                client_id: auth.client_id.clone(),
                inner: TransactionCallbackInfo {
                    transaction_id,
                    inner: TransactionInfo {
                        state: TransactionState::Paid,
                    },
                },
            }]
            .into_iter(),
        )
        .await;

        Ok(Json(CreatePaymentResponseFree { transaction_id }))
    }
    /// You WILL get info on the callback (unless your endpoint is unreachable) about either it
    /// getting cancelled or paid. We will do our best within our control.
    #[oai(path = "/swish", method = "post")]
    async fn swish_payment(
        &self,
        auth: ApiAuth,
        body: Json<CreatePaymentRequest>,
    ) -> MinilithResult<Json<CreatePaymentResponseSwish>> {
        let amount = body.total_amount();
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
        .await?;

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
            .wrap_err_internal("swish api error")?;
        if !resp.status().is_success() {
            let body = resp.text().await;
            return Err(MinilithEndpointError::internal_error(
                "got non 2xx response from swish API",
                body,
            ));
        }
        let prt = resp
            .headers()
            .get("PaymentRequestToken")
            .and_then(|header| header.to_str().ok())
            .wrap_err_internal("swish gave us no PaymentRequestToken")?;

        let mut txn = self.db.begin().await?;
        sqlx::query!(
            "insert into transactions (id, client_id, callback_url_v1,
                total_transaction_fee, callback_identifier)
            values ($1, $2, $3, '1.50'::money, $4)",
            uuid,
            auth.client_id,
            auth.callback_url_v1,
            cb_ident
        )
        .execute(&mut txn.executor())
        .await?;

        insert_wares(&mut txn.executor(), uuid, &body.wares).await?;
        txn.commit().await?;

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
        .await?
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
        .await?
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
        .await?;
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
            amount: row.amount.0,
            tax: row.tax,
            currency: Currency::default(),
        })
        .fetch_all(&self.db)
        .await?;
        let data = receipt::Data {
            language: body.language,
            transaction_id: id.to_string(),
            purchase_date: transaction
                .created
                .date()
                .format(&time::format_description::well_known::Iso8601::DATE)
                .unwrap_or_default(),
            provider: transaction.provider,
            payment_reference,
            refund_reference: transaction.refund_reference,
            wares,
            customer_name: body.customer_name.clone(),
            merchant_id: transaction.client_id,
            merchant_name: client_id.name,
            merchant_org_id: client_id.organization_number,
            merchant_email: client_id.email,
            merchant_address: client_id.address,
            merchant_svg_icon: client_id.svg_icon,
        };
        let doc = receipt::compile(&self.typst_world, &data)?;
        Ok(Response::new(Binary(doc)).header("content-type", "application/octet-stream"))
    }
}
async fn insert_wares(
    executor: impl sqlx::PgExecutor<'_>,
    transaction_id: Uuid,
    wares: &[Ware],
) -> MinilithResult<()> {
    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        reason = "bruh"
    )]
    let idxes = wares
        .iter()
        .enumerate()
        .map(|(idx, _)| idx as i32)
        .collect::<Vec<_>>();
    let transaction_ids = std::iter::repeat_n(transaction_id, wares.len()).collect::<Vec<_>>();
    let names = wares
        .iter()
        .map(|ware| ware.name.clone())
        .collect::<Vec<_>>();
    let amounts = wares
        .iter()
        .map(|ware| PgMoney(ware.amount))
        .collect::<Vec<_>>();
    let currencies = wares
        .iter()
        .map(|ware| ware.currency.to_string())
        .collect::<Vec<_>>();
    let taxes = wares.iter().map(|ware| ware.tax).collect::<Vec<_>>();
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
    .execute(executor)
    .await?;
    Ok(())
}
