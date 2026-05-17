use std::path::PathBuf;

use mini_moka::sync::Cache;
use poem_openapi::Object;
use samael::service_provider::ServiceProvider;

use base64::Engine as _;
use color_eyre::{Section as _, eyre::Context as _};
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use jsonwebtoken::EncodingKey;
use serde::Serialize;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

use crate::{api, saml2};

#[derive(Clone, Debug)]
pub(crate) struct AuthSession {
    pub origin: String,
    pub callback: Option<CallbackUrl>,
    pub continue_url: String,

    pub validated_user: Option<UserData>,
}
#[derive(Serialize, Clone, Debug)]
pub(crate) struct UserData {
    /// User ID.
    pub sub: String,
    pub full_name: String,
    pub email: String,
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
#[derive(Clone, Debug, Object)]
pub struct CallbackUrl {
    v1: String,
}
impl CallbackUrl {
    pub fn as_latest(&self) -> CallbackUrlVersion<'_> {
        CallbackUrlVersion::V1 { url: &self.v1 }
    }
}

#[derive(Clone)]
pub(crate) struct Context {
    // service connections
    pub db: PgPool,
    pub reqwest_client: reqwest::Client,
    pub email: Option<(
        lettre::Address,
        lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    )>,

    // keys
    pub private_key: EncodingKey,
    pub public_key: Vec<u8>,
    pub saml_private_key: openssl::pkey::PKey<openssl::pkey::Private>,

    pub service_provider: ServiceProvider,

    pub auth_sessions: Cache<String, AuthSession>,

    // provider auth data
    pub saml2_request_id_cache: Cache<String, ()>,
    pub email_token_holding: Cache<String, api::EmailLoginRequest>,
}
impl Context {
    fn get_jwt_keys() -> color_eyre::Result<(EncodingKey, Vec<u8>)> {
        let key = std::env::var("PRIVATE_KEY").wrap_err("`PRIVATE_KEY` not detected")?;
        let key = base64::prelude::BASE64_STANDARD
            .decode(key)
            .wrap_err("`PRIVATE_KEY` not base64 encoded")?;
        let ed_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)
            .wrap_err("`PRIVATE_KEY` not valid EdDSA key")?;
        let verifying_key = ed_key.verifying_key();
        let encoding_key = EncodingKey::from_ed_der(&key);
        let public_key = verifying_key.as_bytes().to_vec();
        Ok((encoding_key, public_key))
    }
    async fn setup_db(db_url: &str) -> color_eyre::Result<PgPool> {
        if !Postgres::database_exists(db_url)
            .await
            .wrap_err("Failed to check if database exists")?
        {
            Postgres::create_database(db_url).await?;
        }

        let db = PgPoolOptions::new()
            .max_connections(50)
            .connect(db_url)
            .await
            .wrap_err("Failed to create database pool")?;

        sqlx::migrate!("./migrations")
            .run(&db)
            .await
            .wrap_err("Failed to run migrations")?;

        Ok(db)
    }
    /// # Errors
    ///
    /// Any errors from setting up the context, including any connections errors to any of the
    /// services.
    pub async fn new(db: Option<PgPool>) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        // for tests we only want to attach once
        let _: Result<(), _> = tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            )
            .try_init();

        let (private_key, public_key) = Self::get_jwt_keys()?;

        let db = if let Some(db) = db {
            db
        } else {
            Self::setup_db(&std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?)
                .await
                .wrap_err("Failed to set up the database")
                .suggestion("Start the database with `docker compose up -d`")?
        };

        let (sp, saml_pk) = saml2::get_service_provider().await?;

        let email = if let Ok(url) = std::env::var("SMTP_URL") {
            let user = std::env::var("SMTP_USER").wrap_err("`SMTP_USER` not set")?;
            let password = std::env::var("SMTP_PASSWORD").wrap_err("`SMTP_PASSWORD` not set")?;

            let credentials =
                lettre::transport::smtp::authentication::Credentials::new(user.clone(), password);
            let transport = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::relay(&url)
                .wrap_err("SMTP setup failed")?
                .credentials(credentials)
                .authentication(vec![
                    lettre::transport::smtp::authentication::Mechanism::Plain,
                ])
                .build();
            Some((
                user.parse()
                    .wrap_err("`SMTP_USER` is not a valid email address")?,
                transport,
            ))
        } else {
            None
        };

        let context = Context {
            db,
            reqwest_client: reqwest::Client::new(),
            email,
            private_key,
            public_key,
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
        };
        Ok(context)
    }
}
