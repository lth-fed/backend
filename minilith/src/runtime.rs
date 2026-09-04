use std::sync::Arc;
use std::time::Duration;

use fed_auth_verifier::callbacks::{TransactionCallbackInfo, TransactionInfo};
use minilith_errors::{
    AlertLevel, MinilithErrorResultExt as _, MinilithResult, alert, escape_email_html,
};
use tracing::error;

use crate::accounting;
use crate::push_notifications::{NotificationRow, PushDeviceRow, send_notifications};
use crate::{ContextWrapper, DbInternationalizedString as DIS, report, ticket, transactions};

const NOTIFICATION_POLL_INTERVAL: Duration = Duration::from_secs(10);
const ACCOUNTING_POLL_INTERVAL: Duration = Duration::from_hours(1);

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
            coalesce(
                (select activity_id from activity_notifications
                    where notification_id = notifications.id),
                (select activity_id from activity_buyers_notifications
                    where notification_id = notifications.id),
                (select activity_id from purchased_ticket_notifications
                    where notification_id = notifications.id)
            ) as "activity_id?",
            sender as "sender!: DIS",
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

fn is_accounting_window(now: time::PrimitiveDateTime) -> bool {
    now.weekday() == time::Weekday::Monday && now.hour() >= 17
}

async fn accounting_window(ctx: &ContextWrapper) -> MinilithResult<bool> {
    let now = sqlx::query_scalar!(
        r#"select (now() at time zone 'Europe/Stockholm')
            as "now!: time::PrimitiveDateTime""#,
    )
    .fetch_one(&ctx.db)
    .await?;
    Ok(is_accounting_window(now))
}

async fn check_next_accounting_report(ctx: &ContextWrapper) -> MinilithResult<bool> {
    let Some((email_client, accountant)) = ctx.accounting_email() else {
        return Ok(false);
    };
    let mut transaction = ctx.db.begin().await?;
    let activity_id = sqlx::query_scalar!(
        r#"select id from activities
        where bookkept = false
        and time_end < now() - interval '1 day'
        order by time_end, id
        limit 1
        for update skip locked"#,
    )
    .fetch_optional(&mut transaction.executor())
    .await?;
    let Some(activity_id) = activity_id else {
        transaction.commit().await?;
        return Ok(false);
    };

    let generated = accounting::generate_activity_report(
        ctx,
        activity_id,
        report::Language::Swedish,
        Vec::new(),
        0,
        true,
    )
    .await?;
    let safe_name = escape_email_html(&generated.activity_name);
    email_client
        .send_html_with_pdf(
            "Teknologappen",
            [accountant],
            &format!("Försäljningsrapport – {}", generated.activity_name),
            format!(
                "<p>Här kommer den automatiska försäljningsrapporten för <strong>{safe_name}</strong>. Kvittokopior ligger sist i den bifogade PDF-filen.</p>"
            ),
            format!("forsaljningsrapport-{activity_id}.pdf"),
            generated.pdf,
        )
        .await
        .wrap_err_internal("failed to email automatic accounting report")?;

    sqlx::query!(
        "update activities set bookkept = true where id = $1 and bookkept = false",
        activity_id,
    )
    .execute(&mut transaction.executor())
    .await?;
    transaction.commit().await?;
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

    if ctx.accounting_email().is_some() {
        let accounting_ctx = Arc::clone(ctx);
        tokio::spawn(async move {
            loop {
                match accounting_window(&accounting_ctx).await {
                    Ok(true) => loop {
                        match check_next_accounting_report(&accounting_ctx).await {
                            Ok(true) => {}
                            Ok(false) => break,
                            Err(err) => {
                                alert(
                                    AlertLevel::L2,
                                    "automatic accounting report failed; see logs",
                                );
                                error!(?err, "automatic accounting report failed");
                                break;
                            }
                        }
                    },
                    Ok(false) => {}
                    Err(err) => error!(?err, "failed to check the accounting schedule"),
                }
                tokio::time::sleep(ACCOUNTING_POLL_INTERVAL).await;
            }
        });
    }
}
