use std::ops::Deref;
use std::path::PathBuf;

use base64::Engine as _;
use bin_common::{DebugConfig, PgPool, setup_db};
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
use uuid::Uuid;

use crate::receipt::OurWonderfulTypstWorldBase;
use crate::{Provider, sumup, swish};

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
#[derive(Debug)]
pub struct SumUpClient {
    client: ::sumup::Client,
    pub merchant_code: String,
}
impl Deref for SumUpClient {
    type Target = ::sumup::Client;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}
impl ClientStore<SumUpClient> {
    /// # Errors
    ///
    /// DB, or the client has no `SumUp` merchant.
    pub async fn get(
        &self,
        db: &PgPool,
        client_id: &str,
        sandbox: bool,
    ) -> MinilithResult<impl Deref<Target = SumUpClient>> {
        if let Some(client) = self.0.get(client_id) {
            return Ok(client);
        }
        let row = sqlx::query!(
            "select sumup_api_key, sumup_merchant_code from client_ids where client_id = $1",
            client_id
        )
        .fetch_optional(db)
        .await?
        .wrap_err_internal("sumup: client_id not found")?;
        let (Some(api_key), Some(merchant_code)) = (row.sumup_api_key, row.sumup_merchant_code)
        else {
            return Err(MinilithEndpointError::bad_frontend_code(
                "your client_id isn't set up for SumUp",
                "",
            ));
        };
        let client = self.0.entry(client_id.to_owned()).or_insert(SumUpClient {
            client: ::sumup::Client::default()
                .with_authorization(::sumup::Authorization::api_key(&api_key))
                .with_base_url(if sandbox {
                    sumup::SANDBOX
                } else {
                    sumup::PRODUCTION
                }),
            merchant_code,
        });
        Ok(client.downgrade())
    }
}
#[derive(Debug)]
pub struct Context {
    pub db: PgPool,
    pub debug: DebugConfig,

    pub swish_clients: ClientStore<SwishClient>,
    pub sumup_clients: ClientStore<SumUpClient>,

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
        let debug = DebugConfig::from_env();

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
            .tls_danger_accept_invalid_certs(debug.enabled)
            .build()?;

        let typst_world = OurWonderfulTypstWorldBase::default();

        let context = Self {
            db,
            debug,

            swish_clients: ClientStore::default(),
            sumup_clients: ClientStore::default(),

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
    pub(crate) async fn get_sumup_client(
        &self,
        client_id: &str,
    ) -> MinilithResult<impl Deref<Target = SumUpClient>> {
        self.sumup_clients
            .get(&self.db, client_id, self.debug.enabled)
            .await
    }
    async fn cancel_swish_transaction(
        &self,
        transaction: &CancelTransactionData,
    ) -> MinilithResult<()> {
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
                self.debug.enabled,
            ))
            .header("content-type", "application/json-patch+json")
            .json(&patch)
            .send()
            .await
            .wrap_err_internal(
                "l2: failed to cancel swish payment request due to connection issues",
            );

        // OK: swish api error, ERR: transport error
        let cancel_failure = match cancel_response {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let text = resp
                    .text()
                    .await
                    .wrap_err_internal("l2: failed to read body of cancel swish payment request")
                    .ok();

                if text.as_deref().is_some_and(|text| text.contains("RP04")) {
                    // Swish no longer knows about the request, so it cannot be paid.
                    return Ok(());
                }

                Ok((status, text))
            }
            Err(err) => Err(err),
        };

        // Swish rejects cancelling a request that is already in a terminal state.
        // Reconcile it instead of assuming that every non-cancellable request was paid.
        let current = client
            .get(swish::payment_request_url(
                swish::ApiVersion::V1,
                transaction.id,
                self.debug.enabled,
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
                    return Ok(());
                }
            }
        }

        if cancel_failure
            .as_ref()
            .is_ok_and(|(_, text)| text.as_deref().is_some_and(|text| text.contains("RP07")))
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "user in middle of paying",
                (transaction.id, cancel_failure),
            ));
        }

        // if there was connection error, just continue as normal, try to make new transaction
        if let Ok((status, text)) = &cancel_failure {
            return Err(MinilithEndpointError::internal_error(
                "l1: swish cancel failed due to unknown reasons",
                (status, text),
            ));
        }
        cancel_failure
            .wrap_err_internal("l1: swish cancel transport")
            .map(|_| ())
    }

    /// # Return
    ///
    /// Returns () if cancel is guaranteed successful, applying from the instant this returns.
    pub(crate) async fn cancel_transaction(
        &self,
        transaction: &CancelTransactionData,
    ) -> MinilithResult<()> {
        match transaction.provider {
            Provider::Swish => self.cancel_swish_transaction(transaction).await,
            Provider::Stripe => Err(MinilithEndpointError::internal_error(
                "l2: stripe transactions can no longer be cancelled",
                "",
            )),
            Provider::Sumup => {
                let checkout_id = sqlx::query_scalar!(
                    "select sumup_checkout_id from transactions where id = $1",
                    transaction.id,
                )
                .fetch_optional(&self.db)
                .await?
                .flatten()
                .wrap_err_internal(
                    "l1: no sumup_checkout_id was associated with a sumup transaction",
                )?;
                if sumup::deactivate_checkout(self, &transaction.client_id, &checkout_id).await? {
                    return Ok(());
                }
                // Not cancellable anymore, so find out what actually happened. `check_timeouts`
                // reconciles via `fetch_transaction_info` when this returns an error.
                let checkout =
                    sumup::get_checkout(self, &transaction.client_id, &checkout_id).await?;
                if checkout.status == sumup::CheckoutStatus::Expired {
                    Ok(())
                } else {
                    Err(MinilithEndpointError::bad_frontend_code(
                        "user in middle of paying",
                        (transaction.id, checkout.status),
                    ))
                }
            }
            Provider::Free => Err(MinilithEndpointError::bad_frontend_code(
                "cannot cancel a free transaction",
                "",
            )),
        }
    }
}
