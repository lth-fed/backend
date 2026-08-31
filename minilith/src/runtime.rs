use std::sync::Arc;
use std::time::Duration;

use fed_auth_verifier::callbacks::{TransactionCallbackInfo, TransactionInfo};
use minilith_errors::{AlertLevel, MinilithResult, alert};
use sqlx::postgres::types::PgInterval;
use tracing::{error, info, warn};

use crate::push_notifications::PushSendResult;
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, InternationalizedString as IS,
    MinilithErrorOptionExt as _, ticket, transactions,
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

async fn send_ticket_release_notification(
    ctx: &ContextWrapper,
    minus_start: u8,
    minus_end: u8,
    id: &str,
    mut title: impl FnMut(&IS) -> serde_json::Value,
    description: serde_json::Value,
) -> MinilithResult<()> {
    debug_assert!(minus_start < minus_end, "otherwise none is ever sent");
    let mut txn = ctx.db.begin().await?;
    // this makes it idempotent
    let rows = sqlx::query!(
        "select kind.id, kind.name as \"name!: DIS\"
        from ticket_kinds kind
        where purchasing_available_start > (now() + $2::interval)
        and purchasing_available_start < (now() + $3::interval)
        and max_tickets > 0
        and not exists (
            select 1 from ticket_kind_notifications
            where id = $1 and ticket_kind_id = kind.id
        )
        for update of kind skip locked",
        id,
        PgInterval {
            microseconds: 1000 * 1000 * 60 * i64::from(minus_start),
            days: 0,
            months: 0,
        },
        PgInterval {
            microseconds: 1000 * 1000 * 60 * i64::from(minus_end),
            days: 0,
            months: 0,
        },
    )
    .fetch_all(&mut txn.executor())
    .await?;

    for row in rows {
        // todo: insert into activity notification with send-at key
        sqlx::query!(
            r#"with notif as (
                insert into notifications (id, title, content, send_at)
                values (uuidv4(), $1, $2, now())
                returning id
            )
            insert into ticket_kind_notifications (id, ticket_kind_id, notification_id) 
            select $3, $4, notif.id as notification_id
            from notif"#,
            title(&row.name),
            description,
            id,
            row.id
        )
        .execute(&mut txn.executor())
        .await?;
    }

    txn.commit().await?;

    Ok(())
}

/// Locks and delivers the oldest due notification.
///
/// The row lock is held while messages are sent. Together with `skip locked`, this lets multiple
/// minilith instances process different notifications without intentionally sending the same one.
/// Delivery is still at-least-once if the process exits after a provider accepts a message but
/// before the database transaction commits.
#[allow(
    clippy::too_many_lines,
    reason = "Keeping the recipient rules in one SQL statement makes their precedence explicit."
)]
async fn check_next_notification(ctx: &ContextWrapper) -> MinilithResult<bool> {
    let mut transaction = ctx.db.begin().await?;

    let notification = sqlx::query!(
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

    if !ctx.has_notification_support() {
        warn!(
            notification_id = ?notification.id,
            title = notification.title.resolve_intl("en", ""),
            "push-notification not sent because setup failed"
        );
        sqlx::query!(
            "update notifications set sent = true where id = $1",
            notification.id
        )
        .execute(&mut transaction.executor())
        .await?;
        transaction.commit().await?;
        return Ok(false);
    }

    // The view applies membership visibility and the closest notification setting on each allowed
    // group. Until personalized behavior is defined, both `all` and `personalized` receive every
    // matching notification.
    let devices = sqlx::query!(
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
        where notification_recipients.notification_id = $1"#,
        notification.id,
    )
    .fetch_all(&mut transaction.executor())
    .await?;

    let mut sent = 0_u64;
    let mut failed = 0_u64;
    let mut removed_devices = 0_u64;
    for device in devices {
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
                let removed = sqlx::query!(
                    "delete from push_devices
                    where device_id = $1 and push_token = $2",
                    device.device_id,
                    device.push_token,
                )
                .execute(&mut transaction.executor())
                .await?
                .rows_affected();
                removed_devices += removed;
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

    sqlx::query!(
        "update notifications set sent = true where id = $1",
        notification.id
    )
    .execute(&mut transaction.executor())
    .await?;
    transaction.commit().await?;

    info!(
        notification_id = %notification.id,
        sent,
        failed,
        removed_devices,
        "processed push notification"
    );
    Ok(true)
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
            if let Err(err) = ticket::check_all_tickets(&ticket_ctx.db).await {
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
            // drop since MinilithEndpointError already logs & sends alert
            drop(send_ticket_release_notification(
                &notification_ctx,
                14,
                16,
                "pre-release",
                |title| {
                    serde_json::json!({
                        "sv": format!(
                            "Biljetterna till {} släpps snart!",
                            title.resolve_intl("sv", "")
                        ),
                        "en": format!(
                            "The tickets to {} are released soon!",
                            title.resolve_intl("en", "")
                        ),
                    })
                },
                serde_json::json!({
                    "sv": "Gå in i appen och ställ dig i kö för att få plats.",
                    "en": "The queues are open.",
                }),
            ).await);
            drop(
                send_ticket_release_notification(
                    &notification_ctx,
                    0,
                    1,
                    "release",
                    |_| {
                        serde_json::json!({
                            "sv": "Biljetterna är släppta!",
                            "en": "The tickets are released!",
                        })
                    },
                    serde_json::json!({
                        "sv": "Se om du fått en reservation.",
                        "en": "Check if you got a reservation.",
                    }),
                )
                .await,
            );
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
