use std::sync::Arc;

use minilith_errors::{MinilithErrorResultExt as _, MinilithResult};
use stripe_checkout::CheckoutSessionStatus;
use tracing::error;

use crate::callback::handle_callback_to_us;
use crate::context::{CancelTransactionData, Context};
use crate::{CallbackEvent, CallbackInfo, Provider, TransactionInfo, TransactionState, swish};

/// # Errors
///
/// DB errors.
pub async fn initial_checks(ctx: &Arc<Context>) -> MinilithResult<()> {
    let unpaid_transactions = sqlx::query!(
        "select id, provider as \"provider: Provider\", client_id, stripe_id
        from transactions
        left outer join stripe_checkouts on (stripe_checkouts.transaction_id = id)
        where payment_reference is null"
    )
    .fetch_all(&ctx.db)
    .await?;

    for txn in unpaid_transactions {
        let data = match txn.provider {
            Provider::Swish => {
                let Ok(client) = ctx.get_swish_client(&txn.client_id).await else {
                    continue;
                };
                let resp = match client.get(swish::payment_request_url(txn.id)).send().await {
                    Ok(resp) => resp,
                    Err(err) => {
                        // ALERT LEVEL 2
                        error!(
                            ?err,
                            "failed to fetch swish payment request status due to connection issues"
                        );
                        break;
                    }
                };
                drop(client);
                if !resp.status().is_success() {
                    // ALERT LEVEL 1
                    error!(
                        status_code=%resp.status(),
                        "swish payment request status fetch failed!"
                    );
                    continue;
                }
                match resp.json().await {
                    Ok(data) => data,
                    Err(err) => {
                        // ALERT LEVEL 2
                        error!(
                            ?err,
                            "failed to get body from swish payment request status \
                            due to parsing json / reading body issues"
                        );
                        continue;
                    }
                }
            }
            Provider::Stripe => {
                let Ok(client) = ctx.get_stripe_client(&txn.client_id).await else {
                    continue;
                };

                let Some(session_id) = txn.stripe_id else {
                    // ALERT LEVEL 1
                    error!("stripe_checkouts doesn't have stripe_id for a stripe transaction!");
                    continue;
                };
                let checkout =
                    stripe_checkout::checkout_session::RetrieveCheckoutSession::new(session_id)
                        .send(&*client)
                        .await
                        .wrap_err_internal("stripe: retrieve checkout")?;
                drop(client);
                let paid = checkout.status == Some(CheckoutSessionStatus::Complete);

                swish::Callback {
                    id: txn.id,
                    payment_reference: paid.then(|| checkout.id.as_str().to_owned()),
                    status: if paid {
                        swish::Status::Paid
                    } else {
                        swish::Status::Cancelled
                    },
                    error_message: None,
                }
            }
        };
        handle_callback_to_us(ctx, data, None).await?;
    }

    Ok(())
}

pub fn spawn(ctx: &Arc<Context>) {
    // one runtime task per instance of this, so every function called in `check_timeouts` has to
    // be safe to be called concurrently from all instances of transactions (i.e. we have to write
    // good sql queries)
    let ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        loop {
            if let Err(err) = check_timeouts(&ctx).await {
                error!(?err, "Error from runtime->check_timeouts");
                // ALERT LEVEL 2
            }

            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

/// # Errors
///
/// DB errors and network errors.
pub async fn check_timeouts(ctx: &Context) -> MinilithResult<()> {
    let mut txn = ctx.db.begin().await?;

    let timed_out_transactions = sqlx::query_as!(
        CancelTransactionData,
        "select id, callback_url_v1, client_id, provider as \"provider: Provider\"
        from transactions
        where timeout < now()
        -- paid = false
        and payment_reference is null
        for update"
    )
    .fetch_all(&mut txn.executor())
    .await?;

    let mut cancelled_uuids = Vec::new();
    let mut cancelled = Vec::new();

    for transaction in timed_out_transactions {
        if ctx.cancel_transaction(&transaction).await? {
            cancelled_uuids.push(transaction.id);
            cancelled.push(transaction);
        }
    }

    crate::callback::send_callbacks(
        &ctx.client,
        &ctx.signing_key,
        cancelled.iter().map(|row| CallbackEvent {
            callback_url_v1: row.callback_url_v1.clone(),
            client_id: row.client_id.clone(),
            inner: CallbackInfo {
                transaction_id: row.id,
                inner: TransactionInfo {
                    state: TransactionState::Cancelled,
                },
            },
        }),
    )
    .await;

    sqlx::query!(
        "delete from transactions using unnest($1::uuid[])",
        &cancelled_uuids
    )
    .execute(&mut txn.executor())
    .await?;

    txn.commit().await?;

    Ok(())
}
