use std::path::PathBuf;

use color_eyre::Section as _;
use color_eyre::eyre::Context as _;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use chacha20::ChaCha20;
use chacha20::cipher::KeyIvInit as _;
use chacha20::cipher::StreamCipher as _;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct Context {
    pub db: PgPool,
    encryption_key: [u8; 32],
}

impl Context {
    #[must_use]
    pub fn new(db: PgPool, encryption_key: [u8; 32]) -> Self {
        Context { db, encryption_key }
    }
    /// Encrypts and decrypts any data using chacha20 given a 12 byte nonce.
    ///
    /// ## Example
    /// ```rs
    /// let nonce: [u8; 12] = rand::random();
    /// let data = b"secret data";
    /// let encrypted = context.endecrypt(data, &nonce);
    /// let decrypted = context.endecrypt(&encrypted, &nonce);
    /// ```
    #[must_use]
    pub fn endecrypt(&self, data: &[u8], nonce: &[u8; 12]) -> Vec<u8> {
        let mut cipher = ChaCha20::new(&self.encryption_key.into(), nonce.into());
        let mut buffer = data.to_owned();
        cipher.apply_keystream(&mut buffer);
        buffer
    }
}
impl Context {
    /// # Errors
    ///
    /// Returns any errors stemming from setting up the DB or other services.
    pub async fn new(test_db: Option<PgPool>) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        // For tests, this may be run several times.
        let _: Result<(), _> = tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            )
            .try_init();

        let db = if let Some(db) = test_db {
            db
        } else {
            Self::setup_db(&std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?)
                .await
                .wrap_err("Failed to set up the database")
                .suggestion("Start the database with `docker compose up -d`")?
        };

        let context = Self { db };
        Ok(context)
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
}
