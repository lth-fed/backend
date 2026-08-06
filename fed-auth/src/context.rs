use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use bin_common::{Transaction, setup_db};
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use jsonwebtoken::jwk::JwkSet;
use poem_openapi::Object;
use samael::service_provider::ServiceProvider;

use base64::Engine as _;
use color_eyre::{Section as _, eyre::Context as _};
use jsonwebtoken::EncodingKey;
use minilith_errors::{EmailClient, configure_alert_email};
use serde::{Deserialize, Serialize};
use sqlx::migrate;
use tracing::{error, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

use crate::oidc::CALLBACK_TOKEN_VALID_FOR;
use crate::{PgPool, jwt, saml2};

#[derive(Clone, Debug, sqlx::Type)]
pub(crate) struct AuthSession {
    pub redirect_uri: String,
    pub client_id: String,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub callback: Option<CallbackUrl>,
    // PKCE
    pub code_challenge: String,

    pub datasharing_confirmed: bool,
    pub redirect_requires_datasharing: bool,
}
#[derive(sqlx::Type, Serialize)]
pub struct ValidatedUser {
    // ALSO UPDATE `fed_auth_verifier`
    pub sub: String,
    pub email: Option<String>,
    pub full_name: Option<String>,
    pub lth_guild: Option<String>,
    // ALSO UPDATE `fed_auth_verifier`
}
#[derive(sqlx::Type)]
pub(crate) struct ValidatedAuthSession {
    #[sqlx(flatten)]
    pub session: AuthSession,
    pub user: ValidatedUser,
}
impl Deref for ValidatedAuthSession {
    type Target = AuthSession;
    fn deref(&self) -> &Self::Target {
        &self.session
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
#[derive(Clone, Debug, Object, Deserialize, Serialize, sqlx::Type)]
pub struct CallbackUrl {
    pub callback_url_v1: String,
}
impl CallbackUrl {
    pub fn as_latest(&self) -> CallbackUrlVersion<'_> {
        CallbackUrlVersion::V1 {
            url: &self.callback_url_v1,
        }
    }
}

pub(crate) type ContextWrapper = Arc<Context>;

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
    pub async fn new(test_db: Option<PgPool>) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        let email_client = if test_db.is_some() {
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

    pub(crate) async fn check_has_session(&self, session: &str) -> bool {
        sqlx::query_scalar!(
            "select exists (
                select 1 from sessions where id = $1
        ) as \"exists!\"",
            session
        )
        .fetch_one(&self.db)
        .await
        .unwrap_or(false)
    }
    pub(crate) async fn get_session(
        &self,
        session: &str,
    ) -> Result<Option<AuthSession>, sqlx::Error> {
        sqlx::query!("select * from sessions where id = $1", session)
            .map(|row| AuthSession {
                redirect_uri: row.redirect_uri,
                client_id: row.client_id,
                state: row.state,
                nonce: row.nonce,
                callback: row.callback_url_v1.map(|cb| CallbackUrl {
                    callback_url_v1: cb,
                }),
                code_challenge: row.code_challenge,
                datasharing_confirmed: row.datasharing_confirmed,
                redirect_requires_datasharing: row.redirect_requires_datasharing,
            })
            .fetch_optional(&self.db)
            .await
    }
    pub(crate) async fn get_validated_session(
        &self,
        session: &str,
    ) -> Result<Option<ValidatedAuthSession>, sqlx::Error> {
        sqlx::query!(
            "select * from sessions
            inner join session_validated_users on session_id = id
            where id = $1
            for update",
            session
        )
        .map(|row| ValidatedAuthSession {
            session: AuthSession {
                redirect_uri: row.redirect_uri,
                client_id: row.client_id,
                state: row.state,
                nonce: row.nonce,
                callback: row.callback_url_v1.map(|cb| CallbackUrl {
                    callback_url_v1: cb,
                }),
                code_challenge: row.code_challenge,
                datasharing_confirmed: row.datasharing_confirmed,
                redirect_requires_datasharing: row.redirect_requires_datasharing,
            },
            user: ValidatedUser {
                sub: row.sub,
                email: row.email,
                full_name: row.full_name,
                lth_guild: row.lth_guild,
            },
        })
        .fetch_optional(&self.db)
        .await
    }
    pub(crate) async fn get_remove_validated_session(
        &self,
        txn: &mut Transaction<'_>,
        session: &str,
    ) -> Result<Option<ValidatedAuthSession>, sqlx::Error> {
        let validated_session = sqlx::query!(
            "select * from sessions
            inner join session_validated_users on session_id = id
            where id = $1
            for update",
            session
        )
        .map(|row| ValidatedAuthSession {
            session: AuthSession {
                redirect_uri: row.redirect_uri,
                client_id: row.client_id,
                state: row.state,
                nonce: row.nonce,
                callback: row.callback_url_v1.map(|cb| CallbackUrl {
                    callback_url_v1: cb,
                }),
                code_challenge: row.code_challenge,
                datasharing_confirmed: row.datasharing_confirmed,
                redirect_requires_datasharing: row.redirect_requires_datasharing,
            },
            user: ValidatedUser {
                sub: row.sub,
                email: row.email,
                full_name: row.full_name,
                lth_guild: row.lth_guild,
            },
        })
        .fetch_optional(&mut txn.executor())
        .await?;
        sqlx::query!("delete from sessions where id = $1", session)
            .execute(&mut txn.executor())
            .await?;
        Ok(validated_session)
    }
    pub(crate) async fn validate_session(
        &self,
        session: &str,
        user: &ValidatedUser,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "insert into session_validated_users
            (session_id, sub, email, full_name, lth_guild) values ($1, $2, $3, $4, $5)",
            session,
            user.sub,
            user.email,
            user.full_name,
            user.lth_guild
        )
        .execute(&self.db)
        .await
        .map(|_| ())
    }

    /// # Errors
    ///
    /// They are logged. If we could not send callback to remote.
    pub(crate) async fn provider_callback_next_url(
        &self,
        code: &str,
        session: &ValidatedAuthSession,
    ) -> Result<String, ()> {
        if session.redirect_requires_datasharing && !session.datasharing_confirmed {
            Ok(format!(
                "/confirm-datasharing/?code={code}&provider={}",
                session
                    .redirect_uri
                    .split('/')
                    .nth(2)
                    .unwrap_or(&session.redirect_uri)
            ))
        } else {
            // we're all set to return!
            // but what if we don't have all the info?
            let mut additional_personal_information = false;
            if let Some(cb_url) = &session.callback {
                let token = jwt::encode(
                    &jwt::StandardClaims::new(
                        &session.client_id,
                        CALLBACK_TOKEN_VALID_FOR,
                        &session.user,
                    ),
                    &self.private_key,
                )
                .map_err(|_| ())?;
                match cb_url.as_latest() {
                    CallbackUrlVersion::V1 { url } => {
                        let resp = self
                            .reqwest_client
                            .post(url)
                            .body(token)
                            .send()
                            .await
                            .inspect_err(|err| error!("auth callback POST failed: {err}"))
                            .map_err(|_| ())?;
                        if !resp.status().is_success() {
                            let status = resp.status();
                            let body = resp.text().await.ok();
                            warn!(%status, ?body, "auth callback POST failed");
                            return Err(());
                        }
                        if resp.status() == reqwest::StatusCode::CREATED
                            && session.user.full_name.is_none()
                        {
                            additional_personal_information = true;
                        }
                    }
                }
            } else {
                additional_personal_information = session.user.full_name.is_none();
            }
            if additional_personal_information {
                return Ok(format!(
                    "/personal-information/?code={code}&sub={}",
                    session.user.sub
                ));
            }

            let query_start = if session.redirect_uri.contains('?') {
                '&'
            } else {
                '?'
            };
            let url = if let Some(state) = &session.state {
                let encoded = serde_urlencoded::to_string(state);
                format!(
                    "{}{query_start}code={code}&state={}",
                    session.redirect_uri,
                    encoded.as_deref().unwrap_or(state)
                )
            } else {
                format!("{}{query_start}code={code}", session.redirect_uri)
            };
            Ok(url)
        }
    }
}
