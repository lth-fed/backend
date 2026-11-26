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
    /// A place in the reservation queue (push only); `Reservation` is a reservation itself.
    ReservationQueue,
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
            Self::ReservationQueue => "reservation_queue",
        }
    }

    /// Whether notifications of this kind are stored and listed in the history. The rest are
    /// pushes only: their id names no history entry.
    #[must_use]
    pub(crate) fn is_in_history(self) -> bool {
        matches!(self, Self::Activity | Self::News | Self::TicketRelease)
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

/// Who an activity's notification was addressed to.
#[derive(Enum, Clone, Copy, Debug, Eq, PartialEq)]
#[oai(rename_all = "snake_case")]
pub enum NotificationAudience {
    /// Everyone who can see the activity (`activity_push_notifications`).
    All,
    /// Everyone allowed to buy the released ticket (`ticket_release_push_notifications`).
    Buyers,
    /// Everyone who owns a product for the activity (`purchased_ticket_push_notifications`).
    TicketHolders,
}

impl NotificationAudience {
    fn from_db(audience: &str) -> Option<Self> {
        match audience {
            "all" => Some(Self::All),
            "buyers" => Some(Self::Buyers),
            "ticket_holders" => Some(Self::TicketHolders),
            _ => None,
        }
    }
}

/// A notification as shown to a client, with its read state.
pub(crate) struct ClientNotification {
    pub id: Uuid,
    pub kind: NotificationKind,
    /// What the notification points at: activity id, news id or product id.
    pub target_id: Option<Uuid>,
    /// The activity it is about, for activity news, ticket-holder news and ticket releases.
    pub activity_id: Option<Uuid>,
    /// Who it was addressed to, for the activity kinds.
    pub audience: Option<NotificationAudience>,
    /// The group a news post was published to.
    pub group_id: Option<Uuid>,
    /// The activity's image, the released product's, or the news group's logo.
    pub image_url: Option<String>,
    pub sender: DIS,
    pub title: DIS,
    pub content: DIS,
    pub send_at: OffsetDateTime,
    pub read: bool,
    /// Removed from the inbox by the user.
    pub dismissed: bool,
}

/// The history: notifications from the last six months the user received (eligible and not
/// muted), with read state.
pub(crate) async fn load_notification_history(
    db: &PgPool,
    user_id: &str,
) -> sqlx::Result<Vec<ClientNotification>> {
    load_notifications(db, user_id, None).await
}

/// With `activity`, everything about that activity the user is eligible for, including what they
/// muted: the activity's own record. Otherwise the history (see [`load_notification_history`]).
#[allow(clippy::too_many_lines, reason = "one query and its row mapping")]
pub(crate) async fn load_notifications(
    db: &PgPool,
    user_id: &str,
    activity: Option<Uuid>,
) -> sqlx::Result<Vec<ClientNotification>> {
    sqlx::query!(
        r#"select
            notification.id,
            notification.kind as "kind!: NotificationKind",
            case notification.kind
                when 'news' then news_link.news_id
                when 'ticket_release' then release_link.ticket_id
                else coalesce(activity_link.activity_id, holders_link.activity_id)
            end as "target_id?",
            coalesce(
                activity_link.activity_id,
                holders_link.activity_id,
                release_activity.activity_id
            ) as "activity_id?",
            case
                when activity_link.activity_id is not null then 'all'
                when release_link.ticket_id is not null then 'buyers'
                when holders_link.activity_id is not null then 'ticket_holders'
            end as "audience?",
            news.target_group_id as "group_id?",
            coalesce(activity_image.url, product_image.url, group_logo.url) as "image_url?",
            notification.sender as "sender!: DIS",
            notification.title as "title!: DIS",
            notification.content as "content!: DIS",
            notification.send_at,
            coalesce(
                state.read,
                notification.send_at <= coalesce(watermark.last_full_read, users.creation)
            ) as "read!",
            coalesce(state.dismissed, false) as "dismissed!"
        from notification_recipients recipient
        inner join push_notifications notification
            on notification.id = recipient.notification_id
        inner join users on users.id = recipient.user_id
        left join lateral (
            select activity_id from activity_push_notifications
            where notification_id = notification.id limit 1
        ) activity_link on true
        left join lateral (
            select activity_id from purchased_ticket_push_notifications
            where notification_id = notification.id limit 1
        ) holders_link on true
        left join lateral (
            select ticket_id from ticket_release_push_notifications
            where notification_id = notification.id limit 1
        ) release_link on true
        left join lateral (
            select activity_id from product_activity_assignments
            where product_id = release_link.ticket_id order by activity_id limit 1
        ) release_activity on true
        left join lateral (
            select news_id from news_push_notifications
            where notification_id = notification.id limit 1
        ) news_link on true
        left join news on news.id = news_link.news_id
        left join activities activity on activity.id = coalesce(
            activity_link.activity_id, holders_link.activity_id, release_activity.activity_id)
        left join images activity_image on activity_image.id = activity.image_id
        left join products product on product.id = release_link.ticket_id
        left join images product_image on product_image.id = product.image_id
        left join groups news_group on news_group.id = news.target_group_id
        left join images group_logo on group_logo.id = news_group.logo_id
        left join user_notification_state watermark on watermark.user_id = recipient.user_id
        left join notification_reads state
            on state.user_id = recipient.user_id and state.notification_id = notification.id
        where recipient.user_id = $1
        and notification.sent
        and (
            ($2::uuid is null
                and recipient.receives
                and notification.send_at >= now() - '6 months'::interval)
            or coalesce(
                activity_link.activity_id,
                holders_link.activity_id,
                release_activity.activity_id
            ) = $2
        )
        order by notification.send_at desc, notification.id"#,
        user_id,
        activity,
    )
    .map(|row| ClientNotification {
        id: row.id,
        kind: row.kind,
        target_id: row.target_id,
        activity_id: row.activity_id,
        audience: row
            .audience
            .as_deref()
            .and_then(NotificationAudience::from_db),
        group_id: row.group_id,
        image_url: row.image_url,
        sender: row.sender,
        title: row.title,
        content: row.content,
        send_at: row.send_at,
        read: row.read,
        dismissed: row.dismissed,
    })
    .fetch_all(db)
    .await
}

/// Marks `ids` read for `user_id`, skipping any the user isn't eligible for.
pub(crate) async fn mark_notifications_read(
    db: &PgPool,
    user_id: &str,
    ids: &[Uuid],
) -> sqlx::Result<()> {
    sqlx::query!(
        r#"insert into notification_reads (user_id, notification_id, read)
        select $1, recipient.notification_id, true
        from notification_recipients recipient
        where recipient.user_id = $1
        and recipient.notification_id = any($2)
        on conflict (user_id, notification_id) do update set read = true"#,
        user_id,
        ids,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Marks everything sent up to `up_to` (default: now) as read, including notifications marked
/// unread, and leaves anything newer as it is. Dismissals are kept. The watermark never moves back.
pub(crate) async fn mark_all_notifications_read(
    db: &PgPool,
    user_id: &str,
    up_to: Option<OffsetDateTime>,
) -> sqlx::Result<()> {
    let mut txn = db.begin().await?;
    let watermark = sqlx::query_scalar!(
        r#"insert into user_notification_state (user_id, last_full_read)
        values ($1, least(coalesce($2, now()), now()))
        on conflict (user_id) do update set last_full_read =
            greatest(user_notification_state.last_full_read, excluded.last_full_read)
        returning last_full_read"#,
        user_id,
        up_to,
    )
    .fetch_one(&mut txn.executor())
    .await?;
    // Up to the watermark, read state follows it again: drop the user's own read/unread choices
    // there, and the rows left with nothing to say.
    sqlx::query!(
        r#"update notification_reads state set read = null
        from push_notifications notification
        where state.user_id = $1
        and notification.id = state.notification_id
        and notification.send_at <= $2"#,
        user_id,
        watermark,
    )
    .execute(&mut txn.executor())
    .await?;
    sqlx::query!(
        "delete from notification_reads
        where user_id = $1 and read is null and not dismissed",
        user_id,
    )
    .execute(&mut txn.executor())
    .await?;
    txn.commit().await
}

/// Sets one notification's read and/or dismissed state for `user_id`; `None` leaves that part as
/// it is. Returns false when the user isn't eligible for the notification.
pub(crate) async fn set_notification_state(
    db: &PgPool,
    user_id: &str,
    id: Uuid,
    read: Option<bool>,
    dismissed: Option<bool>,
) -> sqlx::Result<bool> {
    let updated = sqlx::query!(
        r#"insert into notification_reads (user_id, notification_id, read, dismissed)
        select $1, $2, $3, coalesce($4, false)
        where exists (
            select 1 from notification_recipients
            where user_id = $1 and notification_id = $2
        )
        on conflict (user_id, notification_id) do update set
            read = coalesce($3, notification_reads.read),
            dismissed = coalesce($4, notification_reads.dismissed)"#,
        user_id,
        id,
        read,
        dismissed,
    )
    .execute(db)
    .await?
    .rows_affected();
    Ok(updated > 0)
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
                if kind.is_in_history() {
                    payload
                        .add_custom_data("notification_id", &id.to_string())
                        .wrap_err_internal("failed to add notification id to APNs payload")?;
                }
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
                if kind.is_in_history() {
                    data.insert("notification_id".to_owned(), id.to_string());
                }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const ACTIVITY: &str = "50000000-0000-0000-0000-000000000001";
    const EVERYONE: &str = "30000000-0000-0000-0000-000000000007";
    const HOLDERS: &str = "30000000-0000-0000-0000-000000000005";
    const TAGGED_NEWS: &str = "30000000-0000-0000-0000-000000000008";

    fn id(value: &str) -> Uuid {
        Uuid::parse_str(value).unwrap()
    }

    async fn receives(db: &PgPool, notification: &str) -> BTreeMap<String, bool> {
        sqlx::query!(
            r#"select user_id as "user_id!", receives as "receives!"
            from notification_recipients where notification_id = $1"#,
            id(notification),
        )
        .fetch_all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.user_id, row.receives))
        .collect()
    }

    async fn history(db: &PgPool, user_id: &str) -> Vec<ClientNotification> {
        load_notification_history(db, user_id).await.unwrap()
    }

    async fn find(db: &PgPool, user_id: &str, notification: &str) -> ClientNotification {
        history(db, user_id)
            .await
            .into_iter()
            .find(|item| item.id == id(notification))
            .unwrap()
    }

    #[sqlx::test(fixtures("notification_history", "notification_preferences"))]
    async fn activity_news_reaches_everyone_who_can_see_it_unless_muted(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        let expected: BTreeMap<String, bool> = [
            ("eligible", true),
            ("no-settings", true),
            // the bell, off
            ("muted", false),
            // every tag the activity carries is opted out
            ("tags-off", false),
            // ... but the bell, on, wins
            ("tags-off-follows", true),
            // "Important" off, "Party" still on
            ("important-off", true),
        ]
        .into_iter()
        .map(|(user, receives)| (user.to_owned(), receives))
        .collect();
        assert_eq!(receives(&db, EVERYONE).await, expected);

        // Ticket-holder news can't be muted: `owner-overridden` turned the activity's bell off.
        let holders = receives(&db, HOLDERS).await;
        assert_eq!(holders.get("owner-overridden"), Some(&true));
        assert_eq!(holders.get("owner-muted"), Some(&true));
    }

    #[sqlx::test(fixtures("notification_history", "notification_preferences"))]
    async fn history_lists_what_was_received_and_the_activity_lists_everything(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        assert!(
            !history(&db, "muted")
                .await
                .iter()
                .any(|item| item.id == id(EVERYONE)),
            "a muted activity's news stays out of the history"
        );
        let on_activity = load_notifications(&db, "muted", Some(id(ACTIVITY)))
            .await
            .unwrap();
        assert_eq!(
            on_activity.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![id(EVERYONE)],
            "the activity's own list still has it"
        );

        let item = find(&db, "eligible", EVERYONE).await;
        assert_eq!(item.kind, NotificationKind::Activity);
        assert_eq!(item.audience, Some(NotificationAudience::All));
        assert_eq!(item.activity_id, Some(id(ACTIVITY)));
        assert_eq!(item.target_id, Some(id(ACTIVITY)));
        assert_eq!(
            item.image_url.as_deref(),
            Some("https://example.com/logo.png")
        );
        assert!(!item.read);
        assert!(!item.dismissed);

        let holders = find(&db, "owner-overridden", HOLDERS).await;
        assert_eq!(holders.audience, Some(NotificationAudience::TicketHolders));
    }

    #[sqlx::test(fixtures("notification_history", "notification_preferences"))]
    async fn news_is_muted_by_opting_out_of_its_tags(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        let news = receives(&db, TAGGED_NEWS).await;
        assert_eq!(news.get("eligible"), Some(&false));
        assert_eq!(news.get("no-settings"), Some(&true));

        let item = find(&db, "no-settings", TAGGED_NEWS).await;
        assert_eq!(item.kind, NotificationKind::News);
        assert_eq!(item.audience, None);
        assert_eq!(
            item.group_id,
            Some(id("20000000-0000-0000-0000-000000000002"))
        );
    }

    #[sqlx::test(fixtures("notification_history", "notification_preferences"))]
    async fn read_state_overrides_the_watermark_and_read_all_resets_it(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        let user = "eligible";
        let state = |item: ClientNotification| (item.read, item.dismissed);

        assert!(
            set_notification_state(&db, user, id(EVERYONE), Some(true), None)
                .await
                .unwrap()
        );
        assert_eq!(state(find(&db, user, EVERYONE).await), (true, false));
        assert!(
            set_notification_state(&db, user, id(EVERYONE), Some(false), Some(true))
                .await
                .unwrap()
        );
        assert_eq!(state(find(&db, user, EVERYONE).await), (false, true));

        // Read-all up to before it was sent leaves it unread: the user hasn't seen it.
        let sent = find(&db, user, EVERYONE).await.send_at;
        mark_all_notifications_read(&db, user, Some(sent - time::Duration::MINUTE))
            .await
            .unwrap();
        assert_eq!(state(find(&db, user, EVERYONE).await), (false, true));

        // Read-all past it overrides "unread", and keeps the dismissal.
        mark_all_notifications_read(&db, user, None).await.unwrap();
        assert_eq!(state(find(&db, user, EVERYONE).await), (true, true));

        // The watermark never moves back.
        mark_all_notifications_read(&db, user, Some(sent - time::Duration::DAY))
            .await
            .unwrap();
        assert!(find(&db, user, EVERYONE).await.read);

        // A notification the user isn't eligible for can't be given state.
        assert!(
            !set_notification_state(&db, "private-member", id(EVERYONE), Some(true), None)
                .await
                .unwrap()
        );
    }

    #[sqlx::test(fixtures("notification_history", "notification_preferences"))]
    async fn marking_read_overrides_unread(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        let user = "no-settings";
        set_notification_state(&db, user, id(EVERYONE), Some(false), None)
            .await
            .unwrap();
        mark_notifications_read(&db, user, &[id(EVERYONE), id(TAGGED_NEWS)])
            .await
            .unwrap();
        assert!(find(&db, user, EVERYONE).await.read);
        assert!(find(&db, user, TAGGED_NEWS).await.read);
    }
}
