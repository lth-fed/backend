use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use a2::{
    Client as ApnsClient, ClientConfig as ApnsClientConfig, DefaultNotificationBuilder, Endpoint,
    Error as ApnsError, ErrorReason as ApnsErrorReason, NotificationBuilder as _,
    NotificationOptions, PushType,
};
use fcm_service::{FcmMessage, FcmNotification, FcmService, Target};
use fed_auth_verifier::User;
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _, MinilithResult};
use poem_openapi::{Enum, Object, OpenApi, payload::Json};
use sqlx::Type;
use sqlx::types::Uuid;

use crate::context::ContextWrapper;

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
        notification_id: Uuid,
        title: &str,
        content: &str,
    ) -> MinilithResult<PushSendResult> {
        match platform {
            PushPlatform::Ios => {
                let apns_id = notification_id.to_string();
                let payload = DefaultNotificationBuilder::new()
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

#[derive(Enum, Type, Clone, Copy, Debug)]
#[oai(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase", type_name = "push_platform")]
pub enum PushPlatform {
    Ios,
    Android,
}
#[derive(Object)]
struct RegisterRequest {
    platform: PushPlatform,
    push_token: String,
    device_id: Uuid,
}
#[derive(Object)]
struct DeregisterRequest {
    device_id: Uuid,
}

#[OpenApi(prefix_path = "/push")]
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
