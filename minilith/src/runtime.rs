use std::sync::Arc;

use fed_auth_verifier::callbacks::{TransactionCallbackInfo, TransactionInfo};
use minilith_errors::{MinilithErrorResultExt as _, MinilithResult};
use tracing::error;

use crate::{ContextWrapper, ticket};

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
    .await
    .wrap_err_db()?;

    for txn in unpaid_transactions {
        let resp = match ctx
            .transactions_get(format!("/v0/{}", txn.transaction_id))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                // ALERT LEVEL 2
                error!(
                    ?err,
                    "failed to fetch transaction status due to connection issues"
                );
                break;
            }
        };
        if !resp.status().is_success() {
            // ALERT LEVEL 1
            error!(
                status_code=%resp.status(),
                "transaction status fetch failed!"
            );
            continue;
        }
        let data: TransactionInfo = match resp.json().await {
            Ok(data) => data,
            Err(err) => {
                // ALERT LEVEL 2
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

/// # Panics
///
/// If 1 >= 60.
pub fn spawn(ctx: &ContextWrapper) {
    let ctx = Arc::clone(ctx);
    // one runtime task per instance of this, so every function called in `check_all_tickets` has to
    // be safe to be called concurrently from all instances of minilith (i.e. we have to write good
    // sql queries)
    tokio::spawn(async move {
        loop {
            if let Err(err) = ticket::check_all_tickets(&ctx.db).await {
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
}
