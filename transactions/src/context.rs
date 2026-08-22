use std::ops::Deref;
use std::path::PathBuf;

use base64::Engine as _;
use bin_common::{PgPool, setup_db};
use color_eyre::Section as _;
use color_eyre::eyre::WrapErr as _;
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::jwk::JwkSet;
use minilith_errors::{
    EmailClient, MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _,
    MinilithResult, configure_alert_email,
};
use sqlx::migrate;
use stripe_checkout::CheckoutSessionStatus;
use stripe_misc::webhook_endpoint::CreateWebhookEndpointEnabledEvents;
use tracing::error;
use uuid::Uuid;

use crate::api::{DOMAIN, STRIPE_WEBHOOK_PATH};

use crate::receipt::OurWonderfulTypstWorldBase;
use crate::{Provider, swish};

#[derive(Debug)]
pub(crate) struct CancelTransactionData {
    pub id: Uuid,
    pub client_id: String,
    pub callback_url_v1: String,
    pub provider: Provider,
}

#[derive(Debug)]
#[must_use]
pub struct ClientStore<T>(dashmap::DashMap<String, T>);
impl<T> Default for ClientStore<T> {
    fn default() -> Self {
        Self(dashmap::DashMap::default())
    }
}
#[derive(Debug)]
pub struct SwishClient {
    client: reqwest::Client,
    pub number: String,
}
impl Deref for SwishClient {
    type Target = reqwest::Client;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}
impl ClientStore<SwishClient> {
    /// # Errors
    ///
    /// DB & client creation.
    pub async fn get(
        &self,
        db: &PgPool,
        client_id: &str,
    ) -> MinilithResult<impl Deref<Target = SwishClient>> {
        if let Some(client) = self.0.get(client_id) {
            return Ok(client);
        }
        let client = sqlx::query!("select * from client_ids where client_id = $1", client_id)
            .fetch_optional(db)
            .await?
            .wrap_err_internal("client_id not found")?;
        let rustls_buf = format!("{}\n{}", client.swish_key, client.swish_cert);

        let swish_client = reqwest::Client::builder()
            .identity(
                reqwest::Identity::from_pem(rustls_buf.as_bytes())
                    .wrap_err_internal("l1: failed to build client authentication from swish")?,
            )
            .build()
            .wrap_err_internal("l1: Failed to build swish client")?;
        let client = self.0.entry(client_id.to_owned()).or_insert(SwishClient {
            client: swish_client,
            number: client.swish_number,
        });
        Ok(client.downgrade())
    }
}
impl ClientStore<stripe::Client> {
    /// # Errors
    ///
    /// DB & client creation.
    pub async fn get(
        &self,
        db: &PgPool,
        client_id: &str,
    ) -> MinilithResult<impl Deref<Target = stripe::Client>> {
        if let Some(client) = self.0.get(client_id) {
            return Ok(client);
        }
        let client = sqlx::query!("select * from client_ids where client_id = $1", client_id)
            .fetch_optional(db)
            .await?
            .wrap_err_internal("client_id not found")?;

        let Some(stripe_secret) = client.stripe_secret else {
            return Err(MinilithEndpointError::bad_frontend_code(
                "your client_id isn't set up for stripe",
                "",
            ));
        };

        let client = stripe::ClientBuilder::new(&stripe_secret)
            .client_id("fed-transactions".into())
            .build()
            .wrap_err_internal("l1: stripe: ClientBuilder failed")?;

        let webhook_url = format!("{DOMAIN}/v0{STRIPE_WEBHOOK_PATH}");
        let webhooks = stripe_misc::webhook_endpoint::ListWebhookEndpoint::new()
            .limit(100_i64)
            .send(&client)
            .await
            .wrap_err_internal("stripe: list webhooks")?;
        // we need to recreate it to get the secret
        for endpoint in webhooks
            .data
            .iter()
            .filter(|webhook| webhook.url == webhook_url)
        {
            if let Err(error) =
                stripe_misc::webhook_endpoint::DeleteWebhookEndpoint::new(&endpoint.id)
                    .send(&client)
                    .await
            {
                error!(?error, "stripe: Deleting endpoint failed");
            }
        }
        let endpoint = stripe_misc::webhook_endpoint::CreateWebhookEndpoint::new(
            vec![
                CreateWebhookEndpointEnabledEvents::CheckoutSessionCompleted,
                CreateWebhookEndpointEnabledEvents::CheckoutSessionExpired,
            ],
            webhook_url,
        )
        .send(&client)
        .await
        .wrap_err_internal("stripe: add webhook")?;

        sqlx::query!(
            "update client_ids set stripe_endpoint_secret = $1 where client_id = $2",
            endpoint.secret,
            client_id
        )
        .execute(db)
        .await?;

        Ok(self
            .0
            .entry(client_id.to_owned())
            .or_insert(client)
            .downgrade())
    }
}

#[derive(Debug)]
pub struct Context {
    pub db: PgPool,

    pub swish_clients: ClientStore<SwishClient>,
    pub stripe_clients: ClientStore<stripe::Client>,

    // our api to those using us
    pub client: reqwest::Client,
    pub jwks: JwkSet,
    pub signing_key: EncodingKey,

    // refunds typst
    pub typst_world: OurWonderfulTypstWorldBase,
}
impl Context {
    fn get_jwt_keys() -> color_eyre::Result<(EncodingKey, JwkSet)> {
        let key = std::env::var("PRIVATE_KEY").wrap_err("`PRIVATE_KEY` not detected")?;
        let key = base64::prelude::BASE64_STANDARD
            .decode(key)
            .wrap_err("`PRIVATE_KEY` not base64 encoded")?;
        let signing_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)?;
        let encoding_key = EncodingKey::from_ed_der(&key);

        let keys = JwkSet {
            keys: vec![fed_auth_verifier::eddsa_to_jwk(
                &signing_key.verifying_key(),
            )],
        };
        Ok((encoding_key, keys))
    }
    /// # Errors
    ///
    /// Returns any errors stemming from setting up the DB or other services.
    pub async fn new(test_db: Option<PgPool>) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        drop(rustls::crypto::ring::default_provider().install_default());

        if test_db.is_none() {
            configure_alert_email(EmailClient::new("ALERT")?)?;
        }

        let (encoding_key, jwks) = Self::get_jwt_keys()?;

        let db = if let Some(db) = test_db {
            db
        } else {
            setup_db(
                &std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?,
                Some(migrate!("./migrations")),
                24,
            )
            .await
            .wrap_err("Failed to set up the database")
            .suggestion("Start the database with `docker compose up -d`")?
        };

        let client = reqwest::Client::builder()
            .tls_danger_accept_invalid_certs(cfg!(debug_assertions))
            .build()?;

        let typst_world = OurWonderfulTypstWorldBase::default();

        let context = Self {
            db,

            swish_clients: ClientStore::default(),
            stripe_clients: ClientStore::default(),

            client,
            jwks,
            signing_key: encoding_key,

            typst_world,
        };
        Ok(context)
    }

    pub(crate) async fn get_swish_client(
        &self,
        client_id: &str,
    ) -> MinilithResult<impl Deref<Target = SwishClient>> {
        self.swish_clients.get(&self.db, client_id).await
    }
    pub(crate) async fn get_stripe_client(
        &self,
        client_id: &str,
    ) -> MinilithResult<impl Deref<Target = stripe::Client>> {
        self.stripe_clients.get(&self.db, client_id).await
    }

    async fn cancel_swish_transaction(
        &self,
        transaction: &CancelTransactionData,
    ) -> MinilithResult<bool> {
        let patch = vec![swish::PaymentRequestPatch {
            op: swish::PaymentRequestPatchOperation::Replace,
            path: "/status".to_owned(),
            value: "cancelled".to_owned(),
        }];
        let client = self.get_swish_client(&transaction.client_id).await?;
        let cancel_response = client
            .patch(swish::payment_request_url(
                swish::ApiVersion::V1,
                transaction.id,
            ))
            .header("content-type", "application/json-patch+json")
            .json(&patch)
            .send()
            .await
            .wrap_err_internal(
                "l2: failed to cancel swish payment request due to connection issues",
            );

        let cancel_failure = match cancel_response {
            Ok(resp) if resp.status().is_success() => return Ok(true),
            Ok(resp) => {
                let status = resp.status();
                let text = resp
                    .text()
                    .await
                    .wrap_err_internal("l2: failed to read body of cancel swish payment request")
                    .ok();

                if text.as_deref().is_some_and(|text| text.contains("RP04")) {
                    // Swish no longer knows about the request, so it cannot be paid.
                    return Ok(true);
                }

                Some((status, text))
            }
            Err(_) => None,
        };

        // Swish rejects cancelling a request that is already in a terminal state.
        // Reconcile it instead of assuming that every non-cancellable request was paid.
        let current = client
            .get(swish::payment_request_url(
                swish::ApiVersion::V1,
                transaction.id,
            ))
            .send()
            .await
            .wrap_err_internal("l2: failed to reconcile swish payment request after cancel failed");
        drop(client);

        if let Ok(resp) = current {
            let status = resp.status();
            if status.is_success() {
                let data = resp.json::<swish::Callback>().await.wrap_err_internal(
                    "l2: failed to parse swish payment request after cancel failed",
                );
                if matches!(
                    data,
                    Ok(swish::Callback {
                        status: Some(swish::Status::Cancelled),
                        ..
                    })
                ) {
                    return Ok(true);
                }
            }
        }

        // if there was connection error, just continue as normal, try to make new transaction
        if let Some((status, text)) = cancel_failure
            && !text.as_deref().is_some_and(|text| text.contains("RP07"))
        {
            drop(MinilithEndpointError::internal_error(
                "l1: swish cancel failed due to unknown reasons",
                (status, text),
            ));
        }
        Ok(false)
    }

    /// # Return
    ///
    /// Returns `true` if cancel is guaranteed successful, applying from the instant this returns.
    pub(crate) async fn cancel_transaction(
        &self,
        transaction: &CancelTransactionData,
    ) -> MinilithResult<bool> {
        match transaction.provider {
            Provider::Swish => self.cancel_swish_transaction(transaction).await,
            Provider::Stripe => {
                let client = self.get_stripe_client(&transaction.client_id).await?;

                let session_id = sqlx::query_scalar!(
                    "select stripe_id from stripe_checkouts where transaction_id = $1",
                    transaction.id,
                )
                .fetch_optional(&self.db)
                .await?
                .wrap_err_internal(
                    "no stripe_checkouts.stripe_id was associated with a stripe transaction",
                )?;

                let Err(expire_error) =
                    stripe_checkout::checkout_session::ExpireCheckoutSession::new(
                        session_id.as_str(),
                    )
                    .send(&*client)
                    .await
                else {
                    return Ok(true);
                };

                // Stripe deliberately returns an error when an already-expired
                // Checkout Session is expired again. Reconcile the resource
                // state instead of matching Stripe's error text, which also
                // covers a race where Stripe expires it during this request.
                let session = match stripe_checkout::checkout_session::RetrieveCheckoutSession::new(
                    session_id,
                )
                .send(&*client)
                .await
                {
                    Ok(session) => session,
                    Err(reconcile_error) => {
                        tracing::warn!(
                            ?expire_error,
                            ?reconcile_error,
                            "stripe: failed to reconcile checkout after cancel failed"
                        );
                        return Err(expire_error).wrap_err_internal("stripe: cancel");
                    }
                };

                if session.status == Some(CheckoutSessionStatus::Expired) {
                    Ok(true)
                } else {
                    Err(expire_error).wrap_err_internal("stripe: cancel")
                }
            }
            Provider::Free => Ok(false),
        }
    }

    /// # Errors
    ///
    /// Errors from stripe API.
    pub async fn stripe_get_fee(
        &self,
        client_id: &str,
        id: impl Into<stripe_checkout::CheckoutSessionId>,
    ) -> MinilithResult<i64> {
        let client = self.get_stripe_client(client_id).await?;
        let data = stripe_checkout::checkout_session::RetrieveCheckoutSession::new(id)
            // for getting the fee
            .expand(["payment_intent.latest_charge.balance_transaction"].map(str::to_owned))
            .send(&*client)
            .await
            .wrap_err_internal("l1: stripe: fetch session data failed when paid")?;
        drop(client);

        // broooo
        let intent = data
            .payment_intent
            .as_ref()
            .wrap_err_bad_frontend("payment_intent should exist when paid")?;
        let intent = intent
            .as_object()
            .wrap_err_bad_frontend("didn't expand payment_intent")?;
        let charge = intent
            .latest_charge
            .as_ref()
            .wrap_err_bad_frontend("no charge when paid")?;
        let charge = charge
            .as_object()
            .wrap_err_bad_frontend("didn't expand latest_charge")?;
        let balance = charge
            .balance_transaction
            .as_ref()
            .wrap_err_bad_frontend("no balance_transaction when paid")?;
        let balance = balance
            .as_object()
            .wrap_err_bad_frontend("didn't expand balance_transaction")?;
        Ok(balance.fee)
    }
}
