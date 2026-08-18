use std::fmt::Display;
use std::ops::Deref;
use std::sync::Arc;

use fed_auth_verifier::callbacks::{TransactionCallbackInfo, TransactionInfo, TransactionState};
use jsonwebtoken::jwk::JwkSet;
use minilith_errors::{
    MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult,
};
use poem::http::HeaderMap;
use poem_openapi::param::Path;
use poem_openapi::payload::{Binary, Json, Response};
use poem_openapi::{Enum, Object, OpenApi};
use serde::Serialize;
use sqlx::postgres::types::PgMoney;
use stripe_checkout::checkout_session::{
    CreateCheckoutSessionLineItems, CreateCheckoutSessionLineItemsPriceData,
    CreateCheckoutSessionLineItemsPriceDataTaxBehavior, CreateCheckoutSessionPaymentIntentData,
    CreateCheckoutSessionPaymentIntentDataCaptureMethod,
    CreateCheckoutSessionPaymentIntentDataSetupFutureUsage,
    CreateCheckoutSessionPaymentMethodTypes, ProductData,
};
use stripe_checkout::{CheckoutSessionMode, CheckoutSessionStatus};
use stripe_webhook::EventObject;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::context::CancelTransactionData;
use crate::{ApiAuth, CallbackEvent, Context, Provider, callback, receipt, swish};

pub const DOMAIN: &str = "https://transactions.teknologappen.se";
pub const STRIPE_WEBHOOK_PATH: &str = "/stripe-callback";
pub const SWISH_WEBHOOK_PATH: &str = "/swish-callback";

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
// ALSO UPDATE `minilith/transactions.rs`
struct InfoRequest {
    transaction_ids: Vec<Uuid>,
    // ALSO UPDATE `minilith/transactions.rs`
}
#[derive(Debug, Object, Clone)]
// ALSO UPDATE `minilith/transactions.rs`
struct SingleInfoResponse {
    id: Uuid,
    state: TransactionState,
    customer_id: Option<String>,
    total_fees: i64,
    /// `null` if `state == Cancelled`.
    provider: Option<Provider>,
    payment_reference: Option<String>,
    refund_reference: Option<String>,
    // ALSO UPDATE `minilith/transactions.rs`
}

#[derive(Debug, Object, Clone)]
// ALSO UPDATE `minilith/transactions.rs`
struct CreatePaymentRequest {
    /// From `/init`.
    id: Uuid,
    /// When this payment request will be cancelled.
    /// Will try to cancel within 30s.
    timeout: OffsetDateTime,
    /// The list of items to be bought.
    wares: Vec<Ware>,
    /// Used for tracking cards in e.g. Stripe.
    customer_id: Option<String>,
    /// Redirected back when user completes transaction. Required for stripe.
    stripe_success_url: Option<String>,
    // ALSO UPDATE `minilith/transactions.rs`
}
impl CreatePaymentRequest {
    // remove when the SEK restriction is lifted
    fn total_amount(&self) -> i64 {
        self.wares.iter().map(|ware| ware.amount).sum()
    }
}
#[derive(Debug, Object, Clone)]
// ALSO UPDATE `minilith/transactions.rs`
struct CreatePaymentResponseSwish {
    /// See <https://developer.swish.nu/api/payment-request/v2#create-payment-request>.
    payment_request_token: String,
    // ALSO UPDATE `minilith/transactions.rs`
}
#[derive(Debug, Object, Clone)]
// ALSO UPDATE `minilith/transactions.rs`
struct CreatePaymentResponseStripe {
    /// See <https://docs.stripe.com/api/checkout/sessions/object#checkout_session_object-url>.
    redirect_url: String,
    // ALSO UPDATE `minilith/transactions.rs`
}

#[derive(Debug, Object, Clone)]
// ALSO UPDATE `minilith/transactions.rs`
struct ReceiptRequest {
    language: receipt::Language,
    customer_name: String,
    // ALSO UPDATE `minilith/transactions.rs`
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
    /// Returns the JWK used to verify transaction callback JWTs.
    #[oai(path = "/jwks", method = "get")]
    #[allow(clippy::unused_async, reason = "OpenAPI requires async handlers")]
    async fn jwks(&self) -> Response<Json<poem_openapi::types::Any<&JwkSet>>> {
        Response::new(Json(poem_openapi::types::Any(&self.jwks)))
    }

    /// Will return as many infos as ids we received. The order is not guaranteed.
    ///
    /// # Errors
    ///
    /// If any is not associated with your client id, this fails.
    #[oai(path = "/info", method = "post")]
    async fn info(
        &self,
        auth: ApiAuth,
        body: Json<InfoRequest>,
    ) -> MinilithResult<Json<Vec<SingleInfoResponse>>> {
        if body.transaction_ids.is_empty() {
            return Ok(Json(vec![]));
        }
        let transactions = sqlx::query!(
            r#"select payment_reference as "payment_reference?", client_id as "client_id?",
            refund_reference as "refund_reference?",
            total_transaction_fee as "total_transaction_fee?",
            customer_id as "customer_id?", provider as "provider?: Provider",
            --
            t.id as "id!"
            from unnest($1::uuid[]) as t(id)
            left join transactions on transactions.id = t.id"#,
            &body.transaction_ids
        )
        .fetch_all(&self.db)
        .await?;
        for transaction in &transactions {
            if let Some(cid) = &transaction.client_id {
                check_client_id(&auth, cid)?;
            }
        }
        let mapped = transactions.into_iter().map(|row| SingleInfoResponse {
            id: row.id,
            state: if row.client_id.is_none() {
                TransactionState::Cancelled
            } else if row.refund_reference.is_some() {
                TransactionState::Refunded
            } else if row.payment_reference.is_some() {
                TransactionState::Paid
            } else {
                TransactionState::Pending
            },
            customer_id: row.customer_id,
            total_fees: row.total_transaction_fee.map_or(0, |money| money.0),
            provider: row.provider,
            payment_reference: row.payment_reference,
            refund_reference: row.refund_reference,
        });
        Ok(Json(mapped.collect()))
    }
    #[oai(path = "/:id", method = "get")]
    async fn state(
        &self,
        auth: ApiAuth,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<TransactionInfo>> {
        let state = self
            .info(
                auth,
                Json(InfoRequest {
                    transaction_ids: vec![id],
                }),
            )
            .await?
            .0
            .first()
            .wrap_err_internal("info should return as many as it got")?
            .state;
        Ok(Json(TransactionInfo { state }))
    }
    /// From the instant this returns the transaction is guaranteed to be cancelled.
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
    /// Needs to be called before `/free`, `/swish` or `/stripe` to get a uuid to use for those
    /// providers.
    ///
    /// # Errors
    ///
    /// None, only DB
    #[oai(path = "/init", method = "post")]
    async fn get_uuid(&self, _auth: ApiAuth) -> MinilithResult<Json<Uuid>> {
        let uuid = Uuid::new_v4();
        sqlx::query!(
            "insert into transaction_reserved_ids (id)
            values ($1)",
            uuid
        )
        .execute(&self.db)
        .await?;
        Ok(Json(uuid))
    }
    async fn validate_init_id(&self, id: Uuid) -> MinilithResult<Uuid> {
        sqlx::query!(
            "delete from transaction_reserved_ids
            where created < now() - '1 hour'::interval"
        )
        .execute(&self.db)
        .await?;
        let row = sqlx::query!(
            "delete from transaction_reserved_ids
            where id = $1",
            id
        )
        .execute(&self.db)
        .await?;
        if row.rows_affected() != 1 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "get an ID from /init first.",
                "",
            ));
        }
        Ok(id)
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
    ) -> MinilithResult<()> {
        let amount = body.total_amount();
        if amount != 0 {
            return Err(MinilithEndpointError::bad_user_input(
                "amount",
                "",
                "this transaction is not for 0SEK!",
                "amount",
            ));
        }

        let transaction_id = self.validate_init_id(body.id).await?;
        let mut txn = self.db.begin().await?;
        sqlx::query!(
            "insert into transactions (id, customer_id, client_id, callback_url_v1,
                timeout, provider, total_transaction_fee, callback_identifier, payment_reference)
            values ($1, $2, $3, $4, $5, 'free'::provider, '0.00'::money, $6, 'free')
            ",
            transaction_id,
            body.customer_id,
            auth.client_id,
            auth.callback_url_v1,
            body.timeout,
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

        Ok(())
    }
    /// You WILL get info on the callback (unless your endpoint is unreachable) about either it
    /// getting cancelled or paid. We will do our best within our control.
    #[oai(path = "/swish", method = "post")]
    async fn swish_payment(
        &self,
        auth: ApiAuth,
        body: Json<CreatePaymentRequest>,
    ) -> MinilithResult<Json<CreatePaymentResponseSwish>> {
        let total_amount = body.total_amount();
        if total_amount < 100 {
            return Err(MinilithEndpointError::bad_user_input(
                "low amount",
                "",
                "amount is less than 1SEK!",
                "amount",
            ));
        }

        let uuid = self.validate_init_id(body.id).await?;
        let cb_ident = Uuid::new_v4();
        let mut amount = total_amount.to_string();
        amount.insert(amount.len() - 2, '.');
        let mut message = body
            .wares
            .iter()
            .fold(String::new(), |acc, ware| (acc + &ware.name) + ", ");
        message.pop();
        message.pop();
        message.retain(|char| "!?(),.-:; åäöÅÄÖ".contains(char) || char.is_ascii_alphanumeric());
        message.truncate(50);
        let resp = {
            let client = self.get_swish_client(&auth.client_id).await?;
            let swish_body = swish::CreatePaymentRequest {
                payee_alias: client.number.clone(),
                amount,
                currency: "SEK".to_owned(),
                callback_url: format!("{DOMAIN}/v0{SWISH_WEBHOOK_PATH}"),
                message,
                callback_identifier: swish::uuid_to_string(cb_ident),
            };
            client
                .put(swish::payment_request_url(swish::ApiVersion::V2, uuid))
                .json(&swish_body)
                .send()
                .await
                .wrap_err_internal("swish api error")?
        };
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
        let fees = sqlx::query!(
            "select swish_payment_fee_fixed, swish_payment_fee_fraction, swish_payment_fee_max
            from client_ids where client_id = $1",
            auth.client_id
        )
        .fetch_one(&mut txn.executor())
        .await?;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            reason = "it's min:ed either way"
        )]
        let fees = fees.swish_payment_fee_fixed
            + PgMoney(
                ((fees.swish_payment_fee_fraction * total_amount as f64 + 1e-6).floor() as i64)
                    .min(fees.swish_payment_fee_max.0),
            );
        sqlx::query!(
            "insert into transactions (id, customer_id, client_id, callback_url_v1,
                timeout, provider, total_transaction_fee, callback_identifier)
            values ($1, $2, $3, $4, $5, 'swish'::provider, $6, $7)",
            uuid,
            body.customer_id,
            auth.client_id,
            auth.callback_url_v1,
            body.timeout,
            fees,
            cb_ident
        )
        .execute(&mut txn.executor())
        .await?;

        insert_wares(&mut txn.executor(), uuid, &body.wares).await?;
        txn.commit().await?;

        Ok(Json(CreatePaymentResponseSwish {
            payment_request_token: prt.to_owned(),
        }))
    }
    /// This endpoint is called by Swish's backend.
    #[oai(path = "/swish-callback", method = "post", hidden = true)]
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
    async fn get_stripe_customer(
        &self,
        stripe_client: &impl Deref<Target = stripe::Client>,
        customer_id: &str,
    ) -> MinilithResult<String> {
        if let Some(id) = sqlx::query_scalar!(
            "select stripe_id from stripe_customers where customer_id = $1",
            customer_id
        )
        .fetch_optional(&self.db)
        .await?
        {
            return Ok(id);
        }

        let customer = stripe_core::customer::CreateCustomer::new()
            .send(&**stripe_client)
            .await
            .wrap_err_internal("stripe: create user")?;
        let stripe_id = customer.id.as_str();
        sqlx::query!(
            "insert into stripe_customers
                (customer_id, stripe_id)
                values ($1, $2)",
            customer_id,
            stripe_id
        )
        .execute(&self.db)
        .await?;

        Ok(stripe_id.to_owned())
    }
    /// You WILL get info on the callback (unless your endpoint is unreachable) about either it
    /// getting cancelled or paid. We will do our best within our control.
    #[oai(path = "/stripe", method = "post")]
    #[allow(
        clippy::too_many_lines,
        reason = "It's linear and well-documented. \
        It's easier to read in its whole than if it was in multiple functions."
    )]
    async fn stripe_payment(
        &self,
        auth: ApiAuth,
        body: Json<CreatePaymentRequest>,
    ) -> MinilithResult<Json<CreatePaymentResponseStripe>> {
        let stripe_success_url = body
            .stripe_success_url
            .as_ref()
            .wrap_err_bad_frontend("stripe_success_url missing")?;

        let amount = body.total_amount();
        if amount < 100 {
            return Err(MinilithEndpointError::bad_user_input(
                "low amount",
                "",
                "amount is less than 1SEK!",
                "amount",
            ));
        }
        let mut session = stripe_checkout::checkout_session::CreateCheckoutSession::new()
            .line_items(
                body.wares
                    .iter()
                    .map(|ware| CreateCheckoutSessionLineItems {
                        quantity: Some(1),
                        price_data: Some(CreateCheckoutSessionLineItemsPriceData {
                            currency: match ware.currency {
                                Currency::Sek => stripe_types::Currency::SEK,
                            },
                            product: None,
                            product_data: Some(ProductData {
                                name: ware.name.clone(),
                                description: None,
                                images: None,
                                metadata: None,
                                tax_code: None,
                                unit_label: None,
                            }),
                            recurring: None,
                            tax_behavior: Some(
                                CreateCheckoutSessionLineItemsPriceDataTaxBehavior::Inclusive,
                            ),
                            unit_amount: Some(ware.amount),
                            unit_amount_decimal: None,
                        }),
                        adjustable_quantity: None,
                        dynamic_tax_rates: None,
                        metadata: None,
                        tax_rates: None,
                        price: None,
                    })
                    .collect::<Vec<_>>(),
            )
            .mode(CheckoutSessionMode::Payment)
            .success_url(stripe_success_url)
            .payment_method_types(vec![CreateCheckoutSessionPaymentMethodTypes::Card])
            .payment_intent_data(CreateCheckoutSessionPaymentIntentData {
                application_fee_amount: None,
                // so we know the fee directly afterwards
                capture_method: Some(
                    CreateCheckoutSessionPaymentIntentDataCaptureMethod::Automatic,
                ),
                description: None,
                metadata: None,
                on_behalf_of: None,
                receipt_email: None,
                // https://docs.stripe.com/payments/payment-intents#future-usage
                setup_future_usage: Some(
                    CreateCheckoutSessionPaymentIntentDataSetupFutureUsage::OnSession,
                ),
                shipping: None,
                statement_descriptor: None,
                statement_descriptor_suffix: None,
                transfer_data: None,
                transfer_group: None,
            });

        let client = self.get_stripe_client(&auth.client_id).await?;
        let customer = if let Some(customer_id) = &body.customer_id {
            Some(self.get_stripe_customer(&client, customer_id).await?)
        } else {
            None
        };

        if let Some(customer) = customer {
            session = session.customer(customer);
        }

        let session = session
            .send(&*client)
            .await
            .wrap_err_internal("stripe: create checkout session")?;
        drop(client);

        let url = session.url.clone().wrap_err_internal("stripe: no url")?;

        let uuid = self.validate_init_id(body.id).await?;

        let mut txn = self.db.begin().await?;

        sqlx::query!(
            "insert into transactions (id, customer_id, client_id, callback_url_v1,
                timeout, provider, total_transaction_fee, callback_identifier)
            values ($1, $2, $3, $4, $5, 'stripe'::provider, '0.00'::money, $6)",
            uuid,
            body.customer_id,
            auth.client_id,
            auth.callback_url_v1,
            body.timeout,
            uuid,
        )
        .execute(&mut txn.executor())
        .await?;

        sqlx::query!(
            "insert into stripe_checkouts (transaction_id, stripe_id)
            values ($1, $2)",
            uuid,
            session.id.as_str()
        )
        .execute(&mut txn.executor())
        .await?;

        insert_wares(&mut txn.executor(), uuid, &body.wares).await?;
        txn.commit().await?;

        Ok(Json(CreatePaymentResponseStripe { redirect_url: url }))
    }
    /// This endpoint is called by Stripe's backend.
    #[oai(path = "/stripe-callback", method = "post", hidden = true)]
    async fn stripe_callback(
        &self,
        body: Binary<Vec<u8>>,
        headers: &HeaderMap,
    ) -> MinilithResult<()> {
        // Stripe sends JSON (`application/json; charset=utf-8`), but the
        // signature must be checked against the exact, unparsed payload.
        let body = String::from_utf8(body.0)
            .wrap_err_bad_frontend("stripe: webhook body is not valid UTF-8")?;
        let signature = headers
            .get("stripe-signature")
            .and_then(|header| header.to_str().ok())
            .wrap_err_bad_frontend("stripe: stripe-signature header not present or valid")?;

        let event = stripe_webhook::Webhook::insecure(&body)
            .wrap_err_bad_frontend("stripe: invalid event")?;
        tracing::warn!(
            "I know it's insecure, we verify the signature later! \
            We need the data to determine the client_id."
        );

        match event.data.object {
            EventObject::CheckoutSessionCompleted(event)
            | EventObject::CheckoutSessionExpired(event) => {
                let Some(row) = sqlx::query!(
                    "select transaction_id, transactions.client_id, stripe_endpoint_secret
                    from stripe_checkouts
                    inner join transactions on (transactions.id = transaction_id)
                    inner join client_ids on (client_ids.client_id = transactions.client_id)
                    where stripe_id = $1",
                    event.id.as_str()
                )
                .fetch_optional(&self.db)
                .await?
                else {
                    // then we don't have it anymore because it was previously cancelled
                    return Ok(());
                };

                let stripe_endpoint_secret = row
                    .stripe_endpoint_secret
                    .wrap_err_bad_frontend("your client_id isn't set up for stripe")?;
                stripe_webhook::Webhook::construct_event(&body, signature, &stripe_endpoint_secret)
                    .wrap_err_bad_frontend("stripe: invalid event")?;

                let status = match event.status {
                    Some(CheckoutSessionStatus::Complete) => Some(swish::Status::Paid),
                    Some(CheckoutSessionStatus::Open) | None => None,
                    _ => Some(swish::Status::Cancelled),
                };

                if status == Some(swish::Status::Paid) {
                    let fee = self.stripe_get_fee(&row.client_id, &event.id).await?;

                    // set so this is idempotent
                    sqlx::query!(
                        "update transactions set total_transaction_fee = $1 where id = $2",
                        PgMoney(fee),
                        row.transaction_id,
                    )
                    .execute(&self.db)
                    .await?;
                }

                let data = swish::Callback {
                    id: row.transaction_id,
                    payment_reference: (status == Some(swish::Status::Paid))
                        .then(|| event.id.as_str().to_owned()),
                    status,
                    error_message: None,
                };
                // callback_identifier not needed since we validate the stripe signature
                callback::handle_callback_to_us(self, data, None).await?;
            }
            // do nothing!
            _ => {}
        }

        Ok(())
    }
    /// Must be made from the same `client_id` as the transaction.
    /// **NOT IMPLEMENTED YET**
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
                "<payment_reference>",
            ));
        }
        if transaction.refund_reference.is_some() {
            return Err(MinilithEndpointError::bad_user_input(
                "refund when refunded",
                "",
                "refund is not available because it's already happened",
                "<refund_reference>",
            ));
        }
        Err(MinilithEndpointError::internal_error("not implemented", ""))

        // Ok(Json(String::new()))
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
            "select id, customer_id, payment_reference, refund_reference,
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
                "<payment_reference>",
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
            customer_name: Some(body.customer_name.clone()),
            customer_id: transaction.customer_id,
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
