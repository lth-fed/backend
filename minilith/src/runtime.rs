use std::sync::Arc;
use std::time::Duration;

use fed_auth_verifier::callbacks::{TransactionCallbackInfo, TransactionInfo};
use minilith_errors::{AlertLevel, MinilithResult, alert};
use sqlx::postgres::types::PgInterval;
use tracing::{error, info, warn};

use crate::push_notifications::PushSendResult;
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, InternationalizedString as IS,
    MinilithErrorOptionExt as _, ticket,
};

const NOTIFICATION_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// TODO: make these only run 1 instance if multiple instances are deployed from cold-start.
///
/// # Errors
///
/// DB errors.
pub async fn initial_checks(ctx: &ContextWrapper) -> MinilithResult<()> {
    let unpaid_transactions = sqlx::query!(
        "select transaction_id as \"transaction_id!\", user_id, ticket_kind_id
        from ticket_reservations
        where transaction_id is not null"
    )
    .fetch_all(&ctx.db)
    .await?;

    for txn in unpaid_transactions {
        let resp = match ctx
            .transactions_get(format!("/v0/{}", txn.transaction_id))
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
                break;
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
            continue;
        }
        let data: TransactionInfo = match resp.json().await {
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
                continue;
            }
        };

        // dumb hack to simulate HTTP request.
        ticket::Router {
            context: Arc::clone(ctx),
        }
        .callback(
            fed_auth_verifier::callbacks::TransactionsCallbackDataV1::single(
                TransactionCallbackInfo {
                    transaction_id: txn.transaction_id,
                    inner: data,
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
    debug_assert!(minus_start > minus_end, "otherwise none is ever sent");
    let mut txn = ctx.db.begin().await?;
    // this makes it idempotent
    let Some(row) = sqlx::query!(
        "select kind.id, kind.name as \"name!: DIS\"
        from ticket_kinds kind
        left outer join ticket_kind_notifications notif
            on notif.ticket_kind_id = kind.id
        where purchasing_available_start > (now() - $2::interval)
        and purchasing_available_start < (now() - $3::interval)
        and notif.id = $1
        and notif.notification_id is null
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
    .fetch_optional(&mut txn.executor())
    .await?
    else {
        return Ok(());
    };

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
        sqlx::query!("delete from notifications where id = $1", notification.id)
            .execute(&mut transaction.executor())
            .await?;
        transaction.commit().await?;
        return Ok(false);
    }

    // Ticket-kind allowed groups determine who can receive the notification. The closest setting
    // on each allowed group or one of its ancestors applies, and no matching setting means no
    // notification. Until personalized behavior is defined, both `all` and `personalized`
    // receive every matching notification.
    let devices = sqlx::query_file!("src/notification-recipients.sql", notification.id)
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

    sqlx::query!("delete from notifications where id = $1", notification.id)
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
/// If 1 >= 60.
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
            // next minute on xx:01
            let mut next = now;
            next += time::Duration::MINUTE;
            #[allow(clippy::unwrap_used, reason = "bruh")]
            next.replace_second(1).unwrap();
            let until = next - now;
            tokio::time::sleep(until.unsigned_abs()).await;
        }
    });

    let notification_ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        loop {
            drop(send_ticket_release_notification(
                &notification_ctx,
                16,
                14,
                "pre-release",
                |title| {
                    serde_json::json!({
                        "sv": format!("Biljetterna till {} släpps snart!", title.resolve_intl("sv", "")),
                        "en": format!("The tickets to {} are released soon!", title.resolve_intl("en", "")),
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
                    1,
                    0,
                    "release",
                    |_| {
                        serde_json::json!({
                            "sv": format!("Biljetterna är släppta!"),
                            "en": format!("The tickets are released!"),
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
