#![allow(clippy::unused_async, reason = "OpenAPI requires async handlers")]
use std::path::PathBuf;

use base64::Engine as _;
use color_eyre::{Section as _, eyre::Context as _};
use ed25519_dalek::pkcs8::{DecodePrivateKey as _, EncodePublicKey as _};
use jsonwebtoken::EncodingKey;
use poem::{Route, Server, listener::TcpListener};
use poem_openapi::payload::{Binary, Json, Response};
use poem_openapi::{ApiResponse, Object, OpenApi, OpenApiService};
use serde::Serialize;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Context {
    pub db: PgPool,
    pub private_key: EncodingKey,
    pub public_key: Vec<u8>,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let key = std::env::var("PRIVATE_KEY").wrap_err("`PRIVATE_KEY` not detected")?;
    let key = base64::prelude::BASE64_STANDARD
        .decode(key)
        .wrap_err("`PRIVATE_KEY` not base64 encoded")?;
    let ed_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)
        .wrap_err("`PRIVATE_KEY` not valid EdDSA key")?;
    let verifying_key = ed_key.verifying_key();
    let encoding_key = EncodingKey::from_ed_der(&key);

    let db = setup_db(&std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?)
        .await
        .wrap_err("Failed to set up the database")
        .suggestion("Start the database with `docker compose up -d`")?;

    let context = Context {
        db,
        private_key: encoding_key,
        public_key: verifying_key
            .to_public_key_der()
            .wrap_err("internal error: failed to encode verifying key to DER")?
            .into_vec(),
    };
    let api_service = OpenApiService::new(
        Router { context },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server("http://localhost:8001/api/v0");
    let ui = api_service.swagger_ui();

    Server::new(TcpListener::bind("localhost:8001"))
        .run(
            Route::new()
                .nest("/api/v0", api_service)
                .nest("/api/v0/docs", ui),
        )
        .await?;

    Ok(())
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

#[derive(Object)]
struct Refresh {
    /// The refresh token.
    refresh_token: Uuid,
    /// The domain for which this token is for.
    domain: String,
}
#[derive(Object)]
struct RefreshResponse {
    refresh_token: Uuid,
    access_token: String,
}
#[derive(ApiResponse)]
enum RefreshError {
    /// Returns when the user either doesn't have a token or the token is invalid.
    #[oai(status = 401)]
    TokenInvalid,
    /// Unknown internal error.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Serialize)]
struct Claims {
    sub: String,
}

#[derive(Clone, Debug)]
pub struct Router {
    pub context: Context,
}
#[OpenApi]
impl Router {
    /// Returns the key as DER.
    #[oai(path = "/verify-key.der", method = "get")]
    async fn get_verify_key(&self) -> Response<Binary<Vec<u8>>> {
        Response::new(Binary(self.context.public_key.clone()))
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/refresh", method = "post")]
    async fn refresh(&self, body: Json<Refresh>) -> Result<Json<RefreshResponse>, RefreshError> {
        let mut conn = self
            .context
            .db
            .begin()
            .await
            .map_err(|_| RefreshError::Unknown)?;
        let get_query = sqlx::query!(
            "select * from auth_refresh_tokens where refresh_token = $1 and domain = $2",
            body.0.refresh_token,
            body.0.domain
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        sqlx::query!(
            "delete from auth_refresh_tokens where refresh_token = $1 and domain = $2",
            body.0.refresh_token,
            body.0.domain
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        let new_refresh = Uuid::new_v4();
        sqlx::query!(
            "insert into auth_refresh_tokens values ($1, $2, $3)",
            get_query.user_id,
            get_query.domain,
            new_refresh
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        conn.commit().await.map_err(|_| RefreshError::Unknown)?;

        let claims = Claims {
            sub: get_query.user_id,
        };
        let access_token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA),
            &claims,
            &self.context.private_key,
        )
        .map_err(|_| RefreshError::Unknown)?;

        Ok(Json(RefreshResponse {
            refresh_token: new_refresh,
            access_token,
        }))
    }
}
