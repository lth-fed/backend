use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use bin_common::setup_db;
use chacha20::ChaCha20;
use chacha20::cipher::KeyIvInit as _;
use chacha20::cipher::StreamCipher as _;
use color_eyre::Section as _;
use color_eyre::eyre::Context as _;
use minilith_errors::{MinilithErrorOptionExt as _, MinilithResult};
use sqlx::migrate;
use uuid::Uuid;

use crate::PgPool;

#[allow(
    clippy::module_name_repetitions,
    reason = "it's imported and should contain the name"
)]
pub type ContextWrapper = Arc<Context>;

#[derive(Debug, Clone)]
pub struct Context {
    pub db: PgPool,
    encryption_key: [u8; 32],

    transactions_api: String,
    transactions_client: reqwest::Client,
    transactions_token: String,
}

impl Context {
    /// # Errors
    ///
    /// Returns any errors stemming from setting up the DB or other services.
    pub async fn new(test_db: Option<PgPool>, migrate: bool) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        let db = if let Some(db) = test_db {
            db
        } else {
            setup_db(
                &std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?,
                migrate.then_some(migrate!("./migrations")),
            )
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

        let transactions_token = std::env::var("TRANSACTIONS_TOKEN")
            .wrap_err("Error: Missing env variable: 'TRANSACTIONS_TOKEN'.")?;

        #[cfg(debug_assertions)]
        let transactions_api = "http://localhost:8002";
        #[cfg(not(debug_assertions))]
        let transactions_api = "https://transactions.teknologappen.se";

        let context = Self {
            db,
            encryption_key,
            transactions_api: transactions_api.to_owned(),
            transactions_client: reqwest::Client::new(),
            transactions_token,
        };
        Ok(context)
    }

    /// Encrypts and decrypts any data using chacha20 given a 12 byte nonce.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use minilith::Context;
    /// # async {
    /// # let context = Context::new(None, false).await.unwrap();
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
    /// # let context = Context::new(None, false).await.unwrap();
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
    /// # let context = Context::new(None, false).await.unwrap();
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
    /// # let context = Context::new(None, false).await.unwrap();
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

    pub fn transactions_get(&self, endpoint: impl AsRef<str>) -> reqwest::RequestBuilder {
        self.transactions_client
            .get(format!("{}{}", self.transactions_api, endpoint.as_ref()))
            .header(
                "authorization",
                format!("Bearer {}", self.transactions_token),
            )
    }
    pub fn transactions_post(&self, endpoint: impl AsRef<str>) -> reqwest::RequestBuilder {
        self.transactions_client
            .post(format!("{}{}", self.transactions_api, endpoint.as_ref()))
            .header(
                "authorization",
                format!("Bearer {}", self.transactions_token),
            )
    }

    /// # Errors
    ///
    /// - user might not be allowed to access this activity
    pub async fn test_activity_access(&self, user: &str, activity_id: &Uuid) -> MinilithResult<()> {
        // this clusterfuck is the same logic as in `./activities.rs`, which checks if this
        // activity should be visible
        sqlx::query!(
            r#"select exists (select 1
            from group_memberships
            inner join groups member_group on member_group.id = group_memberships.group_id
            -- get the ticket_kinds we're allowed to purchase
            inner join groups allowed_group on allowed_group.path @> member_group.path
            inner join ticket_kind_allowed_groups tk_ag on tk_ag.group_id = allowed_group.id
            inner join ticket_kinds kind on kind.id = tk_ag.ticket_kind_id

            where group_memberships.user_id = $1
            and kind.activity_id = $2
            and (
                member_group.limit_membership_visibility = false
                or tk_ag.group_id = group_memberships.group_id
            ))
            "#,
            user,
            activity_id
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_bad_frontend("user not allowed to access this activity")?;
        Ok(())
    }
}
