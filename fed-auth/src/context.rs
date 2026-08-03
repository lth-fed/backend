use std::path::PathBuf;

use bin_common::setup_db;
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use fed_auth_verifier::callbacks::AuthCallbackDataV1;
use jsonwebtoken::jwk::JwkSet;
use mini_moka::sync::Cache;
use poem_openapi::Object;
use samael::service_provider::ServiceProvider;

use base64::Engine as _;
#[cfg(not(debug_assertions))]
use color_eyre::eyre::ContextCompat as _;
use color_eyre::{Section as _, eyre::Context as _};
use jsonwebtoken::EncodingKey;
use minilith_errors::{EmailClient, configure_alert_email};
use serde::{Deserialize, Serialize};
use sqlx::migrate;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

use crate::{PgPool, api, saml2};

#[derive(Clone, Debug)]
pub(crate) struct AuthSession {
    pub redirect_uri: String,
    pub client_id: String,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub callback: Option<CallbackUrl>,
    // PKCE
    pub code_challenge: String,

    pub validated_user: Option<AuthCallbackDataV1>,
    pub datasharing_confirmed: bool,
    pub redirect_requires_datasharing: bool,
}
impl AuthSession {
    pub(crate) fn provider_callback_next_url(&self, code: &str) -> String {
        if self.redirect_requires_datasharing && !self.datasharing_confirmed {
            format!(
                "/confirm-datasharing/?code={code}&provider={}",
                self.redirect_uri
                    .split('/')
                    .nth(2)
                    .unwrap_or(&self.redirect_uri)
            )
        } else {
            let query_start = if self.redirect_uri.contains('?') {
                '&'
            } else {
                '?'
            };
            if let Some(state) = &self.state {
                let encoded = serde_urlencoded::to_string(state);
                format!(
                    "{}{query_start}code={code}&state={}",
                    self.redirect_uri,
                    encoded.as_deref().unwrap_or(state)
                )
            } else {
                format!("{}{query_start}code={code}", self.redirect_uri)
            }
        }
    }
}
pub enum CallbackUrlVersion<'a> {
    V1 { url: &'a str },
}
impl CallbackUrlVersion<'_> {
    pub fn url(&self) -> &str {
        match self {
            Self::V1 { url } => url,
        }
    }
}
#[derive(Clone, Debug, Object, Deserialize, Serialize)]
pub struct CallbackUrl {
    callback_url_v1: String,
}
impl CallbackUrl {
    pub fn as_latest(&self) -> CallbackUrlVersion<'_> {
        CallbackUrlVersion::V1 {
            url: &self.callback_url_v1,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Context {
    // service connections
    pub db: PgPool,
    pub reqwest_client: reqwest::Client,
    pub email_client: Option<EmailClient>,

    // keys
    pub private_key: EncodingKey,
    pub jwks: JwkSet,
    pub saml_private_key: openssl::pkey::PKey<openssl::pkey::Private>,

    pub service_provider: ServiceProvider,

    pub auth_sessions: Cache<String, AuthSession>,

    // provider auth data
    pub saml2_request_id_cache: Cache<String, ()>,
    pub email_token_holding: Cache<String, api::EmailLoginRequest>,

    pub request_counter: opentelemetry::metrics::Counter<u64>,
    pub error_counter: opentelemetry::metrics::Counter<u64>,
}
impl Context {
    fn get_jwt_keys() -> color_eyre::Result<(EncodingKey, JwkSet)> {
        let key = std::env::var("PRIVATE_KEY").wrap_err("`PRIVATE_KEY` not detected")?;
        let key = base64::prelude::BASE64_STANDARD
            .decode(key)
            .wrap_err("`PRIVATE_KEY` not base64 encoded")?;
        let signing_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)?;
        let encoding_key = EncodingKey::from_ed_der(&key);

        let jwk = fed_auth_verifier::eddsa_to_jwk(&signing_key.verifying_key());
        let keys = JwkSet { keys: vec![jwk] };
        Ok((encoding_key, keys))
    }
    /// # Errors
    ///
    /// Any errors from setting up the context, including any connections errors to any of the
    /// services.
    pub async fn new(db: Option<PgPool>) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        let email_client = if db.is_some() {
            None
        } else {
            configure_alert_email(EmailClient::new("ALERT")?)?;
            let email_client = EmailClient::new("MAIL")?;
            #[cfg(not(debug_assertions))]
            let email_client =
                Some(email_client.wrap_err("`MAIL_*` email configuration is required")?);
            email_client
        };

        // for tests we only want to attach once
        let _: Result<(), _> = tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            )
            .try_init();

        let (private_key, jwks) = Self::get_jwt_keys()?;

        let db = if let Some(db) = db {
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

        let (sp, saml_pk) = saml2::get_service_provider().await?;

        // so they are grouped with the usual poem errors:
        // https://docs.rs/poem/latest/src/poem/middleware/opentelemetry_metrics.rs.html
        let meter = opentelemetry::global::meter("poem");
        let context = Context {
            db,
            reqwest_client: reqwest::Client::new(),
            email_client,
            private_key,
            jwks,
            service_provider: sp,
            saml_private_key: saml_pk,
            // keep them for 30 minutes
            saml2_request_id_cache: Cache::builder()
                .time_to_live(std::time::Duration::from_mins(30))
                .build(),
            auth_sessions: Cache::builder()
                .time_to_live(std::time::Duration::from_mins(30))
                .build(),
            email_token_holding: Cache::builder()
                .time_to_live(std::time::Duration::from_mins(30))
                .build(),

            request_counter: meter
                .u64_counter("poem_requests_count")
                .with_description("request count (since start of service)")
                .build(),
            error_counter: meter
                .u64_counter("poem_errors_count")
                .with_description("failed request count (since start of service)")
                .build(),
        };
        Ok(context)
    }
}
