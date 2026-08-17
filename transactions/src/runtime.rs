use std::ops::ControlFlow;
use std::sync::Arc;

use minilith_errors::{
    MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult,
};
use sqlx::postgres::types::PgMoney;
use stripe_checkout::CheckoutSessionStatus;
use uuid::Uuid;

use crate::callback::handle_callback_to_us;
use crate::context::{CancelTransactionData, Context};
use crate::{CallbackEvent, CallbackInfo, Provider, TransactionInfo, TransactionState, swish};

struct TxnData {
    id: Uuid,
    client_id: String,
    provider: Provider,
    stripe_id: Option<String>,
}
/// # Errors
///
/// DB errors.
async fn fetch_transaction_info(ctx: &Context, txn: TxnData) -> MinilithResult<ControlFlow<()>> {
    let data = match txn.provider {
        Provider::Swish => {
            let Ok(client) = ctx.get_swish_client(&txn.client_id).await else {
                return Ok(ControlFlow::Continue(()));
            };
            let Ok(resp) = client
                .get(swish::payment_request_url(swish::ApiVersion::V1, txn.id))
                .send()
                .await
                .wrap_err_internal(
                    "l2: failed to fetch swish payment request status due to connection issues",
                )
            else {
                return Ok(ControlFlow::Break(()));
            };
            drop(client);
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.ok();
                drop(MinilithEndpointError::internal_error(
                    format!("l1: swish payment request status fetch failed! Status: {status}"),
                    body,
                ));
                return Ok(ControlFlow::Continue(()));
            }
            let Ok(data) = resp.json().await.wrap_err_internal(
                "l2: failed to get body from swish payment request status \
                        due to parsing json / reading body issues",
            ) else {
                return Ok(ControlFlow::Continue(()));
            };
            data
        }
        Provider::Stripe => {
            let Ok(client) = ctx.get_stripe_client(&txn.client_id).await else {
                return Ok(ControlFlow::Continue(()));
            };

            let Ok(session_id) = txn.stripe_id.wrap_err_internal(
                "l1: stripe_checkouts doesn't have a stripe_id for a stripe transaction",
            ) else {
                return Ok(ControlFlow::Continue(()));
            };
            let checkout = stripe_checkout::checkout_session::RetrieveCheckoutSession::new(
                session_id.as_str(),
            )
            .send(&*client)
            .await
            .wrap_err_internal("stripe: retrieve checkout")?;
            drop(client);
            let status = match checkout.status {
                Some(CheckoutSessionStatus::Complete) => Some(swish::Status::Paid),
                Some(CheckoutSessionStatus::Open) | None => None,
                _ => Some(swish::Status::Cancelled),
            };
            if status == Some(swish::Status::Paid) {
                let fee = ctx.stripe_get_fee(&txn.client_id, session_id).await?;

                // set so this is idempotent
                sqlx::query!(
                    "update transactions set total_transaction_fee = $1 where id = $2",
                    PgMoney(fee),
                    txn.id,
                )
                .execute(&ctx.db)
                .await?;
            }

            swish::Callback {
                id: txn.id,
                payment_reference: (status == Some(swish::Status::Paid))
                    .then(|| checkout.id.as_str().to_owned()),
                status,
                error_message: None,
            }
        }
        Provider::Free => {
            return Ok(ControlFlow::Continue(()));
        }
    };
    handle_callback_to_us(ctx, data, None).await?;
    Ok(ControlFlow::Continue(()))
}

/// # Errors
///
/// DB errors.
pub async fn initial_checks(ctx: &Arc<Context>) -> MinilithResult<()> {
    crate::fortnox::recover_stale_jobs(ctx).await?;

    let unpaid_transactions = sqlx::query_as!(
        TxnData,
        "select id, provider as \"provider: Provider\", client_id, stripe_id
        from transactions
        left outer join stripe_checkouts on (stripe_checkouts.transaction_id = id)
        where payment_reference is null"
    )
    .fetch_all(&ctx.db)
    .await?;

    for txn in unpaid_transactions {
        match fetch_transaction_info(ctx, txn).await? {
            ControlFlow::Continue(()) => {}
            ControlFlow::Break(()) => break,
        }
    }

    check_timeouts(ctx).await?;

    Ok(())
}

pub fn spawn(ctx: &Arc<Context>) {
    // one runtime task per instance of this, so every function called in `check_timeouts` has to
    // be safe to be called concurrently from all instances of transactions (i.e. we have to write
    // good sql queries)
    let timeout_context = Arc::clone(ctx);
    tokio::spawn(async move {
        loop {
            let res = check_timeouts(&timeout_context)
                .await
                .wrap_err_internal("l2: error from runtime->check_timeouts");
            drop(res);

            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });

    let fortnox_context = Arc::clone(ctx);
    tokio::spawn(async move {
        let mut jobs = tokio::time::interval(std::time::Duration::from_secs(2));
        let mut stale_jobs = tokio::time::interval(std::time::Duration::from_mins(10));
        // `interval` ticks immediately. Startup recovery already ran in `initial_checks`.
        stale_jobs.tick().await;
        loop {
            tokio::select! {
                _ = jobs.tick() => {
                    let res = crate::fortnox::process_next_job(&fortnox_context)
                        .await
                        .wrap_err_internal("l2: error from runtime->process_next_fortnox_job");
                    drop(res);
                }
                _ = stale_jobs.tick() => {
                    let res = crate::fortnox::recover_stale_jobs(&fortnox_context)
                        .await
                        .wrap_err_internal("l2: error from runtime->recover_stale_fortnox_jobs");
                    drop(res);
                }
            }
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
        } else {
            let txn = sqlx::query_as!(
                TxnData,
                "select id, client_id, provider as \"provider: Provider\",
                stripe_id as \"stripe_id?\"
                from transactions
                left outer join stripe_checkouts
                    on (stripe_checkouts.transaction_id = transactions.id)
                where id = $1",
                transaction.id
            )
            .fetch_one(&mut txn.executor())
            .await?;
            let _: ControlFlow<()> = fetch_transaction_info(ctx, txn).await?;
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
        "delete from transactions using unnest($1::uuid[]) as t(id)",
        &cancelled_uuids
    )
    .execute(&mut txn.executor())
    .await?;

    txn.commit().await?;

    Ok(())
}
