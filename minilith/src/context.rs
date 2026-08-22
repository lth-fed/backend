use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use bin_common::setup_db;
use chacha20::ChaCha20;
use chacha20::cipher::KeyIvInit as _;
use chacha20::cipher::StreamCipher as _;
use color_eyre::Section as _;
use color_eyre::eyre::Context as _;
#[cfg(not(debug_assertions))]
use color_eyre::eyre::ContextCompat as _;
use minilith_errors::{
    AlertLevel, EmailClient, MinilithEndpointError, MinilithResult, alert, configure_alert_email,
};
use sqlx::migrate;
use tracing::{error, warn};
use uuid::Uuid;

use crate::push_notifications::{PushClients, PushPlatform, PushSendResult};
use crate::{PgPool, report};

#[allow(
    clippy::module_name_repetitions,
    reason = "it's imported and should contain the name"
)]
pub type ContextWrapper = Arc<Context>;

#[derive(Debug)]
pub struct Context {
    pub db: PgPool,
    encryption_key: [u8; 32],

    transactions_api: String,
    transactions_client: reqwest::Client,
    transactions_token: String,

    push_clients: Option<PushClients>,

    email_client: Option<EmailClient>,
    report_typst: report::OurWonderfulTypstWorldBase,

    s3_image_bucket: Box<s3::Bucket>,
    s3_image_public_url: String,
}

impl Context {
    /// # Errors
    ///
    /// Returns any errors stemming from setting up the DB or other services.
    #[allow(clippy::too_many_lines, reason = "it's very linear and easy to read")]
    pub async fn new(test_db: Option<PgPool>, migrate: bool) -> color_eyre::Result<Self> {
        let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

        let is_test = test_db.is_some();
        let email_client = if is_test {
            None
        } else {
            configure_alert_email(EmailClient::new("ALERT")?)?;
            let email_client = EmailClient::new("MAIL")?;
            #[cfg(not(debug_assertions))]
            let email_client =
                Some(email_client.wrap_err("`MAIL_*` email configuration is required")?);
            email_client
        };

        let db = if let Some(db) = test_db {
            db
        } else {
            setup_db(
                &std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?,
                migrate.then_some(migrate!("./migrations")),
                24,
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
        let transactions_api = "https://localhost:8052";
        #[cfg(not(debug_assertions))]
        let transactions_api = "https://transactions.teknologappen.se";

        let client = reqwest::Client::builder()
            .tls_danger_accept_invalid_certs(cfg!(debug_assertions))
            .build()?;

        let push_clients = if is_test {
            None
        } else {
            match PushClients::from_env().await {
                Ok(None) => {
                    #[cfg(not(debug_assertions))]
                    {
                        alert(
                            AlertLevel::L2,
                            "push-notifications credentials not available",
                        );
                    }
                    warn!(
                        "push-notification sending disabled because provider credentials are not set"
                    );

                    None
                }
                Ok(Some(push_clients)) => match push_clients.verify_credentials().await {
                    Ok(()) => Some(push_clients),
                    Err(error) => {
                        #[cfg(not(debug_assertions))]
                        alert(
                            AlertLevel::L2,
                            "push-notifications credential test failed. See logs",
                        );
                        error!(
                            ?error,
                            "push-notification sending disabled because its credential test failed"
                        );
                        None
                    }
                },
                Err(error) => {
                    #[cfg(not(debug_assertions))]
                    alert(AlertLevel::L2, "push-notifications setup failed. See logs");
                    error!(
                        ?error,
                        "push-notification sending disabled because setup failed"
                    );
                    None
                }
            }
        };

        let s3_access_key = std::env::var("S3_ACCESS_KEY")?;
        let s3_secret_key = std::env::var("S3_SECRET_KEY")?;
        #[cfg(debug_assertions)]
        let s3_url = "http://localhost:9000";
        #[cfg(not(debug_assertions))]
        let s3_url = "http://fed-s3:9000";
        let s3_image_bucket = s3::Bucket::new(
            "image",
            s3::Region::Custom {
                // region: "tappen-1".to_owned(),
                region: "us-east-1".to_owned(),
                endpoint: s3_url.to_owned(),
            },
            s3::creds::Credentials::new(
                Some(&s3_access_key),
                Some(&s3_secret_key),
                None,
                None,
                None,
            )?,
        )?
        .with_path_style();
        let s3_image_public_url = std::env::var("S3_PUBLIC_URL")
            .unwrap_or_else(|_| s3_image_bucket.url())
            .trim_end_matches('/')
            .to_owned();

        if !is_test {
            match s3_image_bucket.exists().await {
                Ok(false) => {
                    alert(AlertLevel::L2, "s3: no image bucket!");
                    warn!("No s3 image bucket exists! Please create one.");
                }
                #[cfg(debug_assertions)]
                Err(error) => {
                    warn!(
                        ?error,
                        "Could not connect to s3 bucket. Continuing without suppoort. \
                    Add account at console: http://localhost:9001 \
                    password&user is rustfsadmin. Save as env vars."
                    );
                }
                res => {
                    res?;
                }
            }
        }

        let context = Self {
            db,
            encryption_key,
            transactions_api: transactions_api.to_owned(),
            transactions_client: client,
            transactions_token,

            push_clients,

            email_client,
            report_typst: report::OurWonderfulTypstWorldBase::default(),

            s3_image_bucket,
            s3_image_public_url,
        };
        Ok(context)
    }

    /// Pads and encrypts any data using chacha20.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use minilith::Context;
    /// # async {
    /// # let context = Context::new(None, false).await.unwrap();
    /// let data = "secret data";
    /// let encrypted = context.encrypt(data);
    /// let decrypted = context.decrypt_string(encrypted).unwrap();
    /// assert_eq!(decrypted, "secret data");
    /// # };
    /// ```
    #[must_use]
    pub fn encrypt(&self, value: &str) -> Vec<u8> {
        let nonce: [u8; 12] = rand::random();
        let mut cipher = ChaCha20::new(&self.encryption_key.into(), &nonce.into());
        let unpadded_len = value.len() + 12 + 8;
        let padded_len = (((unpadded_len - 1) / 64 + 1) * 64).max(unpadded_len);
        let mut buffer = Vec::with_capacity(padded_len);
        buffer.extend_from_slice(&nonce);
        buffer.extend_from_slice(&(value.len() as u64).to_le_bytes());
        buffer.extend_from_slice(value.as_bytes());
        buffer.extend(std::iter::repeat_n(0, padded_len - unpadded_len));
        #[allow(
            clippy::indexing_slicing,
            reason = "we have at least 20 bytes in this vec"
        )]
        cipher.apply_keystream(&mut buffer[12..]);
        buffer
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
    /// let data = "secret data";
    /// let mut encrypted = context.encrypt(data);
    /// let decrypted = context.decrypt_str(&mut encrypted).unwrap();
    /// assert_eq!(decrypted, "secret data");
    /// # };
    /// ```
    #[must_use]
    pub fn decrypt_str<'a>(&self, encrypted_data: &'a mut [u8]) -> Option<&'a str> {
        let nonce = encrypted_data.get(..12)?;
        let nonce: [u8; 12] = nonce.try_into().ok()?;
        let mut cipher = ChaCha20::new(&self.encryption_key.into(), &nonce.into());
        cipher.apply_keystream(encrypted_data.get_mut(12..)?);
        let decrypted_data = encrypted_data;
        let len = decrypted_data.get(12..20)?.try_into().ok()?;
        #[allow(clippy::cast_possible_truncation, reason = "we only run on 64-bit")]
        let len = u64::from_le_bytes(len) as usize;
        let value = decrypted_data.get(20..(20 + len))?;
        str::from_utf8(value).ok()
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
    /// let data = "secret data";
    /// let mut encrypted = context.encrypt(data);
    /// let decrypted = context.decrypt_string(encrypted).unwrap();
    /// assert_eq!(decrypted, "secret data");
    /// # };
    /// ```
    #[must_use]
    pub fn decrypt_string(&self, mut encrypted_data: Vec<u8>) -> Option<String> {
        let nonce = encrypted_data.get(..12)?;
        let nonce: [u8; 12] = nonce.try_into().ok()?;
        let mut cipher = ChaCha20::new(&self.encryption_key.into(), &nonce.into());
        cipher.apply_keystream(encrypted_data.get_mut(12..)?);
        let mut decrypted_data = encrypted_data;
        let len = decrypted_data.get(12..20)?.try_into().ok()?;
        #[allow(clippy::cast_possible_truncation, reason = "we only run on 64-bit")]
        let len = u64::from_le_bytes(len) as usize;
        let value_range = 20..(20 + len);
        decrypted_data.get(value_range.clone())?;
        decrypted_data.copy_within(value_range, 0);
        decrypted_data.truncate(len);
        String::from_utf8(decrypted_data).ok()
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

    #[must_use]
    pub fn has_notification_support(&self) -> bool {
        self.push_clients.is_some()
    }

    #[must_use]
    pub(crate) fn email_client(&self) -> Option<&EmailClient> {
        self.email_client.as_ref()
    }

    /// # Errors
    ///
    /// Returns an internal error if the push provider rejects the notification.
    pub(crate) async fn send_notification(
        &self,
        platform: PushPlatform,
        push_token: &str,
        notification_id: Uuid,
        title: &str,
        content: &str,
    ) -> MinilithResult<PushSendResult> {
        let sender = self.push_clients.as_ref().ok_or_else(|| {
            MinilithEndpointError::internal_error(
                "push-notification clients are not configured",
                "",
            )
        })?;
        sender
            .send(platform, push_token, notification_id, title, content)
            .await
    }

    #[must_use]
    pub fn image_bucket(&self) -> &s3::Bucket {
        &self.s3_image_bucket
    }

    #[must_use]
    pub fn image_public_url(&self) -> &str {
        &self.s3_image_public_url
    }

    pub(crate) fn report_typst(&self) -> &report::OurWonderfulTypstWorldBase {
        &self.report_typst
    }

    /// # Errors
    ///
    /// - user might not be allowed to access this activity
    pub async fn test_activity_access(&self, user: &str, activity_id: &Uuid) -> MinilithResult<()> {
        // this clusterfuck is the same logic as in `./activities.rs`, which checks if this
        // activity should be visible
        let allowed = sqlx::query_scalar!(
            r#"select (
                exists (
                    select 1
                    from group_memberships
                    inner join groups member_group
                        on member_group.id = group_memberships.group_id
                    inner join groups allowed_group
                        on allowed_group.path @> member_group.path
                    inner join ticket_kind_allowed_groups tk_ag
                        on tk_ag.group_id = allowed_group.id
                    inner join ticket_kinds kind on kind.id = tk_ag.ticket_kind_id
                    where group_memberships.user_id = $1
                    and kind.activity_id = $2
                    and (
                        member_group.limit_membership_visibility = false
                        or tk_ag.group_id = group_memberships.group_id
                    )
                )
                or exists (
                    select 1
                    from activity_hosts
                    inner join allow_admins_from_group_view_activities allowed
                        on allowed.host_group_id = activity_hosts.group_id
                    inner join groups allowed_g on allowed_g.id = allowed.access_group_id
                    inner join groups admin_group on admin_group.path <@ allowed_g.path
                    inner join group_adminships
                        on group_adminships.group_id = admin_group.id
                    inner join activities on activities.id = activity_hosts.activity_id
                    where activity_hosts.activity_id = $2
                    and group_adminships.user_id = $1
                    and (activities.is_hidden_for_other_admins = false
                        or activity_hosts.group_id = admin_group.id)
                )
                or exists (
                    select 1
                    from activity_hosts
                    inner join groups host_group on host_group.id = activity_hosts.group_id
                    inner join groups admin_group on admin_group.path <@ host_group.path
                        or admin_group.path @> host_group.path
                    inner join group_adminships
                        on group_adminships.group_id = admin_group.id
                    where activity_hosts.activity_id = $2
                    and group_adminships.user_id = $1
                )
            ) as "exists!""#,
            user,
            activity_id
        )
        .fetch_one(&self.db)
        .await?;
        if allowed {
            Ok(())
        } else {
            Err(MinilithEndpointError::bad_frontend_code(
                "user not allowed to access this activity",
                "",
            ))
        }
    }
}
