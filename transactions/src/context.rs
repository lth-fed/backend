use std::ops::{Deref, Not as _};
use std::path::PathBuf;

use base64::Engine as _;
use bin_common::{PgPool, setup_db};
use color_eyre::Section as _;
use color_eyre::eyre::WrapErr as _;
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::jwk::JwkSet;
use minilith_errors::{
    AlertLevel, MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _,
    MinilithResult, alert,
};
use sqlx::migrate;
use stripe_misc::webhook_endpoint::CreateWebhookEndpointEnabledEvents;
use tracing::error;
use uuid::Uuid;

use crate::api::{DOMAIN, STRIPE_WEBHOOK_PATH};

use crate::receipt::OurWonderfulTypstWorldBase;
use crate::{Provider, swish};

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
                    .wrap_err_internal("failed to build client authentication from swish")
                    .inspect_err(|_| {
                        alert(
                            AlertLevel::L1,
                            format!("swish config for a client_id ({client_id}) is invalid"),
                        );
                    })?,
            )
            .build()
            .wrap_err_internal("Failed to build swish client")
            .inspect_err(|_| {
                alert(
                    AlertLevel::L1,
                    format!("failed to build swish client for client_id {client_id}"),
                );
            })?;
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
            .wrap_err_internal("stripe: ClientBuilder failed")
            .inspect_err(|_| {
                alert(AlertLevel::L1, "stripe ClientBuilder failed");
            })?;

        let webhook_url = format!("{DOMAIN}/v0{STRIPE_WEBHOOK_PATH}");
        let webhooks = stripe_misc::webhook_endpoint::ListWebhookEndpoint::new()
            .limit(100_i64)
            .send(&client)
            .await
            .wrap_err_internal("stripe: list webhooks")?;
        let should_add_webhook = webhooks
            .data
            .iter()
            .any(|webhook| webhook.url == webhook_url)
            .not();
        if should_add_webhook {
            stripe_misc::webhook_endpoint::CreateWebhookEndpoint::new(
                vec![
                    CreateWebhookEndpointEnabledEvents::CheckoutSessionCompleted,
                    CreateWebhookEndpointEnabledEvents::CheckoutSessionExpired,
                ],
                webhook_url,
            )
            // for getting the fee
            .expand(
                [
                    "payment_intent",
                    "payment_intent.latest_charge",
                    "payment_intent.latest_charge.balance_transaction",
                    "latest_charge",
                    "balance_transaction",
                ]
                .map(str::to_owned),
            )
            .send(&client)
            .await
            .wrap_err_internal("stripe: add webhook")?;
        }

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
        let _signing_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)?;
        let encoding_key = EncodingKey::from_ed_der(&key);

        // let jwk = fed_auth_verifier::eddsa_to_jwk(&signing_key.verifying_key());
        let keys = JwkSet {
            keys: vec![/*jwk*/],
        };
        Ok((encoding_key, keys))
    }
    /// # Errors
    ///
    /// Returns any errors stemming from setting up the DB or other services.
    pub async fn new(test_db: Option<PgPool>) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        let (encoding_key, jwks) = Self::get_jwt_keys()?;

        let db = if let Some(db) = test_db {
            db
        } else {
            setup_db(
                &std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?,
                Some(migrate!("./migrations")),
            )
            .await
            .wrap_err("Failed to set up the database")
            .suggestion("Start the database with `docker compose up -d`")?
        };

        let typst_world = OurWonderfulTypstWorldBase::default();

        let context = Self {
            db,

            swish_clients: ClientStore::default(),
            stripe_clients: ClientStore::default(),

            client: reqwest::Client::new(),
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

    /// # Return
    ///
    /// Returns `true` if cancel is guaranteed successful, applying from the instant this returns.
    pub(crate) async fn cancel_transaction(
        &self,
        transaction: &CancelTransactionData,
    ) -> MinilithResult<bool> {
        match transaction.provider {
            Provider::Swish => {
                let patch = vec![swish::PaymentRequestPatch {
                    op: swish::PaymentRequestPatchOperation::Replace,
                    path: "/status".to_owned(),
                    value: "cancelled".to_owned(),
                }];
                let client = self.get_swish_client(&transaction.client_id).await?;
                let resp = match client
                    .patch(swish::payment_request_url(
                        swish::ApiVersion::V1,
                        transaction.id,
                    ))
                    .header("content-type", "application/json-patch+json")
                    .json(&patch)
                    .send()
                    .await
                {
                    Ok(resp) => resp,
                    Err(err) => {
                        alert(
                            AlertLevel::L2,
                            "failed to cancel swish payment request due to connection issues",
                        );
                        error!(
                            ?err,
                            "failed to cancel swish payment request due to connection issues"
                        );
                        return Ok(false);
                    }
                };
                drop(client);
                let status = resp.status();
                if status.is_success() {
                    return Ok(true);
                }

                let text = match resp.text().await {
                    Ok(text) => text,
                    Err(err) => {
                        alert(
                            AlertLevel::L2,
                            "failed to read body of cancel swish payment request",
                        );
                        error!(?err, "failed to read body of cancel swish payment request");
                        return Ok(false);
                    }
                };

                if text.contains("RP04") {
                    // TXN not found, it's obviously cancelled
                    println!("rp04");
                    return Ok(true);
                }
                // non-cancellable state
                if text.contains("RP07") {
                    return Ok(false);
                }

                error!(
                    error_body = text,
                    status_code = ?status,
                    "Shit went down with swish cancel!"
                );
                alert(
                    AlertLevel::L1,
                    "swish cancel failed due to unknown reasons (see logs for error_body & status)",
                );
                Ok(false)
            }
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

                stripe_checkout::checkout_session::ExpireCheckoutSession::new(session_id)
                    .send(&*client)
                    .await
                    .wrap_err_internal("stripe: cancel")?;
                Ok(true)
            }
            Provider::Free => Ok(false),
        }
    }
}
