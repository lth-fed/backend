use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use a2::{
    Client as ApnsClient, ClientConfig as ApnsClientConfig, DefaultNotificationBuilder, Endpoint,
    Error as ApnsError, ErrorReason as ApnsErrorReason, NotificationBuilder as _,
    NotificationOptions, PushType,
};
use bin_common::Transaction;
use fcm_service::{FcmMessage, FcmNotification, FcmService, Target};
use fed_auth_verifier::User;
use minilith_errors::{
    AlertLevel, MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _,
    MinilithResult, alert,
};
use poem_openapi::{Enum, Object, OpenApi, payload::Json};
use sqlx::Type;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;
use tracing::{error, info, warn};

use crate::context::ContextWrapper;
use crate::{DbInternationalizedString as DIS, PgPool};

#[derive(Clone)]
pub(crate) struct PushClients {
    apns: ApnsClient,
    apns_topic: String,
    fcm: Arc<FcmService>,
}

impl fmt::Debug for PushClients {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PushClients")
            .field("apns", &self.apns)
            .field("apns_topic", &self.apns_topic)
            .field("fcm", &"FcmService")
            .finish()
    }
}

#[derive(Enum, Type, Clone, Copy, Debug)]
#[oai(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase", type_name = "push_platform")]
pub enum PushPlatform {
    Ios,
    Android,
}

#[derive(Enum, Type, Clone, Copy, Debug, Eq, PartialEq)]
#[oai(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case", type_name = "notification_kind")]
pub enum NotificationKind {
    Activity,
    News,
    TicketRelease,
    Transfer,
    Reservation,
    /// Admins refunded a purchase.
    Refund,
}

impl NotificationKind {
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::News => "news",
            Self::TicketRelease => "ticket_release",
            Self::Transfer => "transfer",
            Self::Reservation => "reservation",
            Self::Refund => "refund",
        }
    }
}

pub(crate) struct PushDeviceRow {
    pub user_id: String,
    pub device_id: String,
    pub push_token: String,
    pub platform: PushPlatform,
    pub language: Vec<u8>,
}

/// The provider-agnostic parts of one push message.
pub(crate) struct PushMessage<'a> {
    pub id: Uuid,
    pub kind: NotificationKind,
    pub activity_id: Option<Uuid>,
    pub title: &'a str,
    pub content: &'a str,
}
pub(crate) struct NotificationRow {
    pub id: Uuid,
    pub kind: NotificationKind,
    pub activity_id: Option<Uuid>,
    pub sender: DIS,
    pub title: DIS,
    pub content: DIS,
}

/// A notification as shown to a client, with its read state.
pub(crate) struct ClientNotification {
    pub id: Uuid,
    pub kind: NotificationKind,
    /// What the notification points at: activity id, news id or product id.
    pub target_id: Option<Uuid>,
    /// Only set for activity notifications; kept for the legacy DTO.
    pub activity_id: Option<Uuid>,
    pub sender: DIS,
    pub title: DIS,
    pub content: DIS,
    pub send_at: OffsetDateTime,
    pub read: bool,
}

/// Notifications the user is currently eligible for, with read state.
pub(crate) async fn load_notification_history(
    db: &PgPool,
    user_id: &str,
) -> sqlx::Result<Vec<ClientNotification>> {
    sqlx::query!(
        r#"select
            notification.id,
            notification.kind as "kind!: NotificationKind",
            case notification.kind
                when 'news' then (
                    select news_id from news_push_notifications
                    where notification_id = notification.id)
                when 'ticket_release' then (
                    select ticket_id from ticket_release_push_notifications
                    where notification_id = notification.id)
                else coalesce(
                    (select activity_id from activity_push_notifications
                        where notification_id = notification.id),
                    (select activity_id from purchased_ticket_push_notifications
                        where notification_id = notification.id))
            end as "target_id?",
            coalesce(
                (select activity_id from activity_push_notifications
                    where notification_id = notification.id),
                (select activity_id from purchased_ticket_push_notifications
                    where notification_id = notification.id)
            ) as "activity_id?",
            notification.sender as "sender!: DIS",
            notification.title as "title!: DIS",
            notification.content as "content!: DIS",
            notification.send_at,
            (
                exists (
                    select 1 from notification_reads read
                    where read.notification_id = notification.id
                    and read.user_id = $1
                )
                or notification.send_at <= coalesce(
                    state.last_full_read,
                    (select creation from users where users.id = $1)
                )
            ) as "read!"
        from notification_recipients
        inner join push_notifications notification
            on notification.id = notification_recipients.notification_id
        left join user_notification_state state on state.user_id = $1
        where notification_recipients.user_id = $1
        and notification.sent = true
        and notification.send_at >= now() - '6 months'::interval
        order by notification.send_at desc, notification.id"#,
        user_id,
    )
    .map(|row| ClientNotification {
        id: row.id,
        kind: row.kind,
        target_id: row.target_id,
        activity_id: row.activity_id,
        sender: row.sender,
        title: row.title,
        content: row.content,
        send_at: row.send_at,
        read: row.read,
    })
    .fetch_all(db)
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PushSendResult {
    Sent,
    InvalidToken,
}

impl PushClients {
    pub(crate) async fn from_env() -> MinilithResult<Option<Self>> {
        const REQUIRED_VARIABLES: [&str; 5] = [
            "FCM_SERVICE_ACCOUNT_PATH",
            "APNS_PRIVATE_KEY_PATH",
            "APNS_KEY_ID",
            "APNS_TEAM_ID",
            "APNS_TOPIC",
        ];

        if REQUIRED_VARIABLES
            .iter()
            .all(|name| std::env::var_os(name).is_none())
        {
            return Ok(None);
        }

        let required = |name: &str| {
            std::env::var(name).wrap_err_internal(format!(
                "`{name}` must be set when any push-notification configuration is present"
            ))
        };

        let fcm_credentials = required("FCM_SERVICE_ACCOUNT_PATH")?;
        let apns_key_path = required("APNS_PRIVATE_KEY_PATH")?;
        let apns_key_id = required("APNS_KEY_ID")?;
        let apns_team_id = required("APNS_TEAM_ID")?;
        let apns_topic = required("APNS_TOPIC")?;
        let apns_endpoint = match std::env::var("APNS_ENDPOINT")
            .unwrap_or_else(|_| "production".to_owned())
            .as_str()
        {
            "production" => Endpoint::Production,
            "sandbox" => Endpoint::Sandbox,
            value => {
                return Err(MinilithEndpointError::internal_error(
                    "`APNS_ENDPOINT` must be `production` or `sandbox`",
                    value,
                ));
            }
        };

        let apns_key = tokio::fs::read(&apns_key_path)
            .await
            .wrap_err_internal(format!("failed to open APNs key at `{apns_key_path}`"))?;
        let apns = ApnsClient::token(
            std::io::Cursor::new(apns_key),
            apns_key_id,
            apns_team_id,
            ApnsClientConfig::new(apns_endpoint),
        )
        .wrap_err_internal("failed to create APNs client")?;

        Ok(Some(Self {
            apns,
            apns_topic,
            fcm: Arc::new(FcmService::new(fcm_credentials)),
        }))
    }

    /// Verifies both provider configurations without delivering to a real device.
    ///
    /// APNs and FCM require a target on every send. The deliberately invalid targets below
    /// therefore must be rejected as targets after authentication succeeds.
    pub(crate) async fn verify_credentials(&self) -> MinilithResult<()> {
        let (apns, fcm) = tokio::join!(
            self.verify_apns_credentials(),
            self.verify_fcm_credentials()
        );
        apns?;
        fcm?;
        Ok(())
    }

    async fn verify_apns_credentials(&self) -> MinilithResult<()> {
        const APNS_TEST_TOKEN: &str =
            "0000000000000000000000000000000000000000000000000000000000000000";

        let payload = DefaultNotificationBuilder::new()
            .set_title("Credential check")
            .set_body("This notification has no recipient.")
            .build(
                APNS_TEST_TOKEN,
                NotificationOptions {
                    apns_id: Some("00000000-0000-0000-0000-000000000000"),
                    apns_push_type: Some(PushType::Alert),
                    apns_topic: Some(&self.apns_topic),
                    ..NotificationOptions::default()
                },
            );
        match self.apns.send(payload).await {
            Err(ApnsError::ResponseError(response))
                if response
                    .error
                    .as_ref()
                    .is_some_and(|error| error.reason == ApnsErrorReason::BadDeviceToken) => {}
            Ok(_) => {
                return Err(MinilithEndpointError::internal_error(
                    "APNs accepted the credential-test target",
                    "the synthetic token unexpectedly accepted a notification",
                ));
            }
            Err(error) => return Err(error).wrap_err_internal("APNs credential test failed"),
        }

        Ok(())
    }

    async fn verify_fcm_credentials(&self) -> MinilithResult<()> {
        let mut notification = FcmNotification::new();
        notification.set_title("Credential check".to_owned());
        notification.set_body("This notification has no recipient.".to_owned());
        let mut message = FcmMessage::new();
        message.set_notification(Some(notification));
        message.set_target(Target::Token("credential-test-no-device".to_owned()));
        match self.fcm.send_notification(message).await {
            Err(error) if fcm_test_target_rejected(&error.to_string()) => {}
            Ok(()) => {
                return Err(MinilithEndpointError::internal_error(
                    "FCM accepted the credential-test target",
                    "the synthetic token unexpectedly accepted a notification",
                ));
            }
            Err(error) => return Err(error).wrap_err_internal("FCM credential test failed"),
        }

        Ok(())
    }

    pub(crate) async fn send(
        &self,
        platform: PushPlatform,
        push_token: &str,
        message: &PushMessage<'_>,
    ) -> MinilithResult<PushSendResult> {
        let PushMessage {
            id,
            kind,
            activity_id,
            title,
            content,
        } = *message;
        match platform {
            PushPlatform::Ios => {
                let apns_id = id.to_string();
                let mut payload = DefaultNotificationBuilder::new()
                    .set_title(title)
                    .set_body(content)
                    .set_sound("default")
                    .build(
                        push_token,
                        NotificationOptions {
                            apns_id: Some(&apns_id),
                            apns_push_type: Some(PushType::Alert),
                            apns_topic: Some(&self.apns_topic),
                            ..NotificationOptions::default()
                        },
                    );
                payload
                    .add_custom_data("kind", &kind.as_str())
                    .wrap_err_internal("failed to add kind to APNs payload")?;
                if let Some(activity_id) = activity_id {
                    payload
                        .add_custom_data("activity_id", &activity_id.to_string())
                        .wrap_err_internal("failed to add activity id to APNs payload")?;
                }
                match self.apns.send(payload).await {
                    Ok(_) => {}
                    Err(ApnsError::ResponseError(response))
                        if response.error.as_ref().is_some_and(|error| {
                            matches!(
                                error.reason,
                                ApnsErrorReason::Unregistered | ApnsErrorReason::BadDeviceToken
                            )
                        }) =>
                    {
                        return Ok(PushSendResult::InvalidToken);
                    }
                    Err(err) => {
                        return Err(err).wrap_err_internal("APNs rejected the notification");
                    }
                }
            }
            PushPlatform::Android => {
                let mut notification = FcmNotification::new();
                notification.set_title(title.to_owned());
                notification.set_body(content.to_owned());

                let mut message = FcmMessage::new();
                message.set_notification(Some(notification));
                let mut data = HashMap::from([("kind".to_owned(), kind.as_str().to_owned())]);
                if let Some(activity_id) = activity_id {
                    data.insert("activity_id".to_owned(), activity_id.to_string());
                }
                message.set_data(Some(data));
                message.set_target(Target::Token(push_token.to_owned()));

                if let Err(err) = self.fcm.send_notification(message).await {
                    let error = err.to_string();
                    if error.contains("UNREGISTERED")
                        || error.contains("registration-token-not-registered")
                    {
                        return Ok(PushSendResult::InvalidToken);
                    }
                    return Err(err).wrap_err_internal("FCM rejected the notification");
                }
            }
        }

        Ok(PushSendResult::Sent)
    }
}

fn fcm_test_target_rejected(error: &str) -> bool {
    error.contains("UNREGISTERED")
        || (error.contains("INVALID_ARGUMENT") && error.contains("registration token"))
}

pub(crate) struct PushDevices {
    pub device_ids: Vec<String>,
    pub push_tokens: Vec<String>,
}
impl PushDevices {
    pub async fn clear_failed(&self, txn: &mut Transaction<'_>) -> MinilithResult<()> {
        sqlx::query!(
            "delete from push_devices pd
            using unnest($1::text[], $2::text[]) as t(device_id, push_token)
            where pd.device_id = t.device_id and pd.push_token = t.push_token",
            &self.device_ids,
            &self.push_tokens,
        )
        .execute(&mut txn.executor())
        .await?;
        Ok(())
    }
}
pub(crate) async fn send_notifications(
    ctx: &ContextWrapper,
    notification: &NotificationRow,
    rows: impl IntoIterator<Item = PushDeviceRow>,
) -> MinilithResult<PushDevices> {
    if !ctx.has_notification_support() {
        warn!(
            notification_id = ?notification.id,
            title = notification.title.resolve_intl("en", ""),
            "push-notification not sent because setup failed"
        );
        return Ok(PushDevices {
            device_ids: Vec::new(),
            push_tokens: Vec::new(),
        });
    }

    let mut sent = 0_u64;
    let mut failed = 0_u64;
    let mut removed_devices = PushDevices {
        device_ids: Vec::new(),
        push_tokens: Vec::new(),
    };
    for device in rows {
        let language = ctx
            .decrypt_string(device.language)
            .wrap_err_encryption("notification recipient language")?;

        let sender = notification.sender.resolve_intl(&language, "");
        let title = notification.title.resolve_intl(&language, "");
        let title = if sender.is_empty() {
            title.to_owned()
        } else if title.trim().is_empty() {
            sender.to_owned()
        } else {
            format!("{sender}: {title}")
        };
        let content = notification.content.resolve_intl(&language, "");
        match ctx
            .send_notification(
                device.platform,
                &device.push_token,
                &PushMessage {
                    id: notification.id,
                    kind: notification.kind,
                    activity_id: notification.activity_id,
                    title: &title,
                    content,
                },
            )
            .await
        {
            Ok(PushSendResult::Sent) => sent += 1,
            Ok(PushSendResult::InvalidToken) => {
                removed_devices.device_ids.push(device.device_id);
                removed_devices.push_tokens.push(device.push_token);
            }
            Err(err) => {
                if failed == 0 {
                    alert(
                        AlertLevel::L3,
                        format!("failed to send push notification (id: {})", notification.id),
                    );
                }
                failed += 1;
                error!(
                    ?err,
                    notification_id = %notification.id,
                    user_id = %device.user_id,
                    device_id = %device.device_id,
                    platform = ?device.platform,
                    "failed to send push notification"
                );
            }
        }
    }
    info!(
        notification_id = %notification.id,
        sent,
        failed,
        removed_devices=removed_devices.push_tokens.len(),
        "processed push notification"
    );
    Ok(removed_devices)
}

#[derive(Clone, Debug)]
pub struct Router {
    pub context: ContextWrapper,
}
impl Deref for Router {
    type Target = ContextWrapper;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Object)]
struct RegisterRequest {
    platform: PushPlatform,
    push_token: String,
    device_id: String,
}
#[derive(Object)]
struct DeregisterRequest {
    device_id: String,
}

#[OpenApi(prefix_path = "/v0/push")]
impl Router {
    /// # Errors
    ///
    /// DB.
    #[oai(path = "/register", method = "post")]
    async fn register_device(&self, user: User, body: Json<RegisterRequest>) -> MinilithResult<()> {
        let mut transaction = self.db.begin().await?;

        // A provider token also identifies one installation. Remove an obsolete installation row
        // before assigning the token and device ID to the currently authenticated user.
        #[allow(trivial_casts, reason = "sqlx")]
        sqlx::query!(
            "delete from push_devices
            where platform = $1
            and push_token = $2
            and device_id != $3",
            body.platform as PushPlatform,
            &body.push_token,
            body.device_id,
        )
        .execute(&mut transaction.executor())
        .await?;

        #[allow(trivial_casts, reason = "sqlx")]
        sqlx::query!(
            "insert into push_devices
            (user_id, device_id, push_token, platform)
            values ($1, $2, $3, $4)
            on conflict (device_id) do update set
            user_id = excluded.user_id,
            push_token = excluded.push_token,
            platform = excluded.platform,
            updated_at = now()",
            user.get_id(),
            body.device_id,
            &body.push_token,
            body.platform as PushPlatform
        )
        .execute(&mut transaction.executor())
        .await?;

        transaction.commit().await?;
        Ok(())
    }
    /// Call this before logging out.
    ///
    /// # Errors
    ///
    /// DB.
    #[oai(path = "/deregister", method = "post")]
    async fn deregister_device(
        &self,
        auth: User,
        body: Json<DeregisterRequest>,
    ) -> MinilithResult<()> {
        sqlx::query!(
            "delete from push_devices
            where device_id = $1 and user_id = $2",
            body.device_id,
            auth.get_id()
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }
}
