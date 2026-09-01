use std::sync::Arc;
use std::time::Duration;

use bin_common::Transaction;
use fed_auth_verifier::callbacks::{TransactionCallbackInfo, TransactionInfo};
use minilith_errors::{AlertLevel, MinilithResult, alert};
use tracing::{error, info, warn};

use crate::push_notifications::{NotificationRow, PushDeviceRow, PushSendResult};
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, MinilithErrorOptionExt as _, ticket,
    transactions,
};

const NOTIFICATION_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// TODO: make these only run 1 instance if multiple instances are deployed from cold-start.
///
/// # Errors
///
/// DB errors.
pub async fn initial_checks(ctx: &ContextWrapper) -> MinilithResult<()> {
    check_unpaid_transactions(ctx).await
}
async fn check_unpaid_transactions(ctx: &ContextWrapper) -> MinilithResult<()> {
    let unpaid_transactions = sqlx::query_scalar!(
        "select transaction_id as \"transaction_id!\"
        from ticket_reservations
        where transaction_id is not null"
    )
    .fetch_all(&ctx.db)
    .await?;

    let resp = match ctx
        .transactions_post("/v0/info")
        .json(&transactions::InfoRequest {
            transaction_ids: unpaid_transactions,
        })
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            alert(
                AlertLevel::L2,
                "connection issues for transaction API when starting up to check txns",
            );
            error!(
                ?err,
                "failed to fetch transaction status due to connection issues"
            );
            return Ok(());
        }
    };
    if !resp.status().is_success() {
        alert(AlertLevel::L1, "transaction status != 200, see logs");
        let status = resp.status();
        let body = resp.text().await;
        error!(
            ?body,
            status_code=%status,
            "transaction status fetch failed!"
        );
        return Ok(());
    }
    let data: Vec<transactions::SingleInfoResponse> = match resp.json().await {
        Ok(data) => data,
        Err(err) => {
            alert(
                AlertLevel::L2,
                "transaction initial fetch failed to parse JSON",
            );
            error!(
                ?err,
                "failed to get body from transaction status \
                    due to parsing json / reading body issues"
            );
            return Ok(());
        }
    };
    // dumb hack to simulate HTTP request.
    let router = ticket::Router {
        context: Arc::clone(ctx),
    };
    for info in data {
        router
            .callback(
                fed_auth_verifier::callbacks::TransactionsCallbackDataV1::single(
                    TransactionCallbackInfo {
                        transaction_id: info.id,
                        inner: TransactionInfo { state: info.state },
                    },
                ),
            )
            .await?;
    }

    Ok(())
}

/// Locks and delivers the oldest due notification.
///
/// The row lock is held while messages are being prepared to be sent. Together with `skip locked`,
/// this lets multiple minilith instances process different notifications without intentionally
/// sending the same one. This is not robust; if the service crashes at most 1 notification is sent.
/// But Minilith doesn't crash 😎.
#[allow(
    clippy::too_many_lines,
    reason = "Keeping the recipient rules in one SQL statement makes their precedence explicit."
)]
async fn check_next_notification(ctx: &ContextWrapper) -> MinilithResult<bool> {
    let mut transaction = ctx.db.begin().await?;

    let notification = sqlx::query_as!(
        NotificationRow,
        r#"select
            id,
            title as "title!: DIS",
            content as "content!: DIS"
        from notifications
        where send_at <= now()
        and sent = false
        order by send_at, id
        for update skip locked"#
    )
    .fetch_optional(&mut transaction.executor())
    .await?;

    let Some(notification) = notification else {
        transaction.commit().await?;
        return Ok(false);
    };

    // The view applies membership visibility and the closest notification setting on each allowed
    // group. Until personalized behavior is defined, both `all` and `personalized` receive every
    // matching notification.
    let devices = sqlx::query_as!(
        PushDeviceRow,
        r#"select
            push_devices.user_id as "user_id!",
            push_devices.device_id as "device_id!",
            push_devices.push_token,
            push_devices.platform::push_platform
                as "platform!: crate::push_notifications::PushPlatform",
            users.language
        from notification_recipients
        inner join push_devices using (user_id)
        inner join users on users.id = notification_recipients.user_id
        where notification_recipients.notification_id = $1
        and notification_level = 'all'::notification_level"#,
        notification.id,
    )
    .fetch_all(&mut transaction.executor())
    .await?;
    sqlx::query!(
        "update notifications set sent = true where id = $1",
        notification.id
    )
    .execute(&mut transaction.executor())
    .await?;

    transaction.commit().await?;

    let removed_devices = send_notifications(ctx, &notification, devices).await?;

    let mut txn = ctx.db.begin().await?;

    removed_devices.clear_failed(&mut txn).await?;

    txn.commit().await?;

    Ok(true)
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

        let title = notification.title.resolve_intl(&language, "");
        let content = notification.content.resolve_intl(&language, "");
        match ctx
            .send_notification(
                device.platform,
                &device.push_token,
                notification.id,
                title,
                content,
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

/// We can scale this, just call it several times.
///
/// # Panics
///
/// If 58 >= 60 or if 0 > 1e9.
pub fn spawn(ctx: &ContextWrapper) {
    let ticket_ctx = Arc::clone(ctx);
    // one runtime task per instance of this, so every function called in `check_all_tickets` has to
    // be safe to be called concurrently from all instances of minilith (i.e. we have to write good
    // sql queries)
    tokio::spawn(async move {
        loop {
            if let Err(err) = ticket::check_all_tickets(&ticket_ctx).await {
                error!(?err, "Error from runtime->check_all_tickets");
            }

            let now = time::OffsetDateTime::now_utc();
            // next minute on xx:58
            // TODO(frontend-hack: 25/08/2026): revert to :00, this is because the frontend jitters at :00 - :10 and in :00 -
            // :01 we may still be actively releasing it.
            #[allow(clippy::unwrap_used, reason = "bruh")]
            let mut next = now
                .replace_second(58)
                .unwrap()
                .replace_nanosecond(0)
                .unwrap();
            if next <= now {
                next += time::Duration::MINUTE;
            }
            let until = next - now;
            tokio::time::sleep(until.unsigned_abs()).await;
        }
    });

    let unpaid_ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        loop {
            if check_unpaid_transactions(&unpaid_ctx).await.is_err() {
                error!("Error from runtime->check_unpaid_transactions");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    let notification_ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        loop {
            loop {
                match check_next_notification(&notification_ctx).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(err) => {
                        error!(?err, "error while checking for push notifications");
                        break;
                    }
                }
            }
            tokio::time::sleep(NOTIFICATION_POLL_INTERVAL).await;
        }
    });
}
