use std::path::PathBuf;

use base64::Engine as _;
use chacha20::ChaCha20;
use chacha20::cipher::KeyIvInit as _;
use chacha20::cipher::StreamCipher as _;
use color_eyre::Section as _;
use color_eyre::eyre::Context as _;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone)]
pub struct Context {
    pub db: PgPool,
    encryption_key: [u8; 32],
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
        let encryption_key = std::env::var("ENCRYPTION_KEY")
            .wrap_err("Error: Missing env variable: 'ENCRYPTION_KEY'.")?;
        let encryption_key = base64::engine::general_purpose::STANDARD
            .decode(encryption_key)
            .wrap_err("Error: Could not parse env variable 'ENCRYPTION_KEY' as base64.")?;
        let encryption_key = <[u8; 32]>::try_from(encryption_key.as_slice()).wrap_err(
            "Error: Env variable 'ENCRYPTION_KEY' is of wrong size. Expected: 32 bytes.",
        )?;

        let context = Self { db, encryption_key };
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

    /// Encrypts and decrypts any data using chacha20 given a 12 byte nonce.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use minilith::Context;
    /// # async {
    /// # let context = Context::new(None).await.unwrap();
    /// let nonce: [u8; 12] = rand::random();
    /// let data = b"secret data";
    /// let encrypted = context.endecrypt(data, &nonce);
    /// let decrypted = context.endecrypt(&encrypted, &nonce);
    /// assert_eq!(decrypted, b"secret data");
    /// # };
    /// ```
    #[must_use]
    pub fn endecrypt(&self, data: &[u8], nonce: &[u8; 12]) -> Vec<u8> {
        let mut cipher = ChaCha20::new(&self.encryption_key.into(), nonce.into());
        let mut buffer = Vec::with_capacity(data.len() + 32);
        buffer.extend_from_slice(data);
        cipher.apply_keystream(&mut buffer);
        buffer
    }
    /// Encrypts and decrypts a mutable byte slice using chacha20 given a 12 byte nonce.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use minilith::Context;
    /// # async {
    /// # let context = Context::new(None).await.unwrap();
    /// let nonce: [u8; 12] = rand::random();
    /// let mut data = Vec::from(b"secret data");
    /// context.endecrypt_mut_slice(&mut data, &nonce);
    /// context.endecrypt_mut_slice(&mut data, &nonce);
    /// assert_eq!(data, b"secret data");
    /// # };
    /// ```
    pub fn endecrypt_mut_slice(&self, data: &mut [u8], nonce: &[u8; 12]) {
        let mut cipher = ChaCha20::new(&self.encryption_key.into(), nonce.into());
        cipher.apply_keystream(data);
    }
    /// Decrypts to a [`&str`] using chacha20 given a nonce (which has to be 12 bytes).
    ///
    /// # Errors
    ///
    /// If the data was not UTF-8, this returns an error. Also errors if `nonce.len() != 12`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use minilith::Context;
    /// # async {
    /// # let context = Context::new(None).await.unwrap();
    /// let nonce: [u8; 12] = rand::random();
    /// let data = b"secret data";
    /// let mut encrypted = context.endecrypt(data, &nonce);
    /// let decrypted = context.decrypt_str(&mut encrypted, &nonce).unwrap();
    /// assert_eq!(decrypted, "secret data");
    /// # };
    /// ```
    #[must_use]
    pub fn decrypt_str<'a>(&self, encrypted_data: &'a mut [u8], nonce: &[u8]) -> Option<&'a str> {
        self.endecrypt_mut_slice(encrypted_data, nonce.try_into().ok()?);
        std::str::from_utf8(encrypted_data).ok()
    }
    /// Decrypts to a [`String`] using chacha20 given a 12 byte nonce.
    ///
    /// # Errors
    ///
    /// If the data was not UTF-8, this returns an error.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use minilith::Context;
    /// # async {
    /// # let context = Context::new(None).await.unwrap();
    /// let nonce: [u8; 12] = rand::random();
    /// let data = b"secret data";
    /// let mut encrypted = context.endecrypt(data, &nonce);
    /// let decrypted = context.decrypt_string(encrypted, &nonce).unwrap();
    /// assert_eq!(decrypted, "secret data");
    /// # };
    /// ```
    #[must_use]
    pub fn decrypt_string(&self, mut encrypted_data: Vec<u8>, nonce: &[u8]) -> Option<String> {
        self.endecrypt_mut_slice(&mut encrypted_data, nonce.try_into().ok()?);
        String::from_utf8(encrypted_data).ok()
    }
}
