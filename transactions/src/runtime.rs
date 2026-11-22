use std::ops::ControlFlow;
use std::sync::Arc;

use minilith_errors::{
    AlertLevel, MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _,
    MinilithResult, alert,
};
use uuid::Uuid;

use crate::callback::{self, handle_callback_to_us};
use crate::context::{CancelTransactionData, Context};
use crate::{
    CallbackEvent, CallbackInfo, Provider, TransactionInfo, TransactionState, sumup, swish,
};

struct TxnData {
    id: Uuid,
    client_id: String,
    provider: Provider,
    sumup_checkout_id: Option<String>,
    sumup_reader_id: Option<String>,
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
                .get(swish::payment_request_url(
                    swish::ApiVersion::V1,
                    txn.id,
                    ctx.debug.enabled,
                ))
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
        Provider::Sumup => {
            let Ok(checkout_id) = txn
                .sumup_checkout_id
                .as_deref()
                .wrap_err_internal("l1: sumup transaction without a checkout id")
            else {
                return Ok(ControlFlow::Continue(()));
            };
            let (status, payment_reference) = if let Some(reader_id) =
                txn.sumup_reader_id.as_deref()
            {
                let Ok((status, client_transaction_id)) =
                    sumup::get_reader_checkout(ctx, &txn.client_id, reader_id, checkout_id).await
                else {
                    return Ok(ControlFlow::Continue(()));
                };
                let payment_reference = if status == sumup::CheckoutStatus::Paid {
                    sumup::transaction_id_for_client_transaction(
                        ctx,
                        &txn.client_id,
                        &client_transaction_id,
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                (status, payment_reference)
            } else {
                let Ok(checkout) = sumup::get_checkout(ctx, &txn.client_id, checkout_id).await
                else {
                    return Ok(ControlFlow::Continue(()));
                };
                let payment_reference =
                    checkout.transaction_id().unwrap_or(&checkout.id).to_owned();
                (checkout.status, Some(payment_reference))
            };
            let status = match status {
                sumup::CheckoutStatus::Paid => Some(swish::Status::Paid),
                sumup::CheckoutStatus::Expired | sumup::CheckoutStatus::Failed => {
                    Some(swish::Status::Cancelled)
                }
                sumup::CheckoutStatus::Pending => None,
            };
            swish::Callback {
                id: txn.id,
                payment_reference: (status == Some(swish::Status::Paid))
                    .then(|| payment_reference.unwrap_or_default())
                    .filter(|reference| !reference.is_empty()),
                status,
                error_message: None,
            }
        }
        // Legacy Stripe rows have nothing left to poll; free payments are instantly paid.
        Provider::Stripe | Provider::Free => {
            return Ok(ControlFlow::Continue(()));
        }
    };
    handle_callback_to_us(ctx, data, None).await?;
    Ok(ControlFlow::Continue(()))
}

/// Re-polls the transaction owning `checkout_id` and tells the client about any state change.
///
/// Used by `/sumup-callback`: `SumUp` POSTs the checkout id of a checkout that changed state, but
/// the payload itself is never trusted — the state always comes from a fresh `SumUp` fetch.
///
/// # Errors
///
/// DB errors and `SumUp` API errors.
pub(crate) async fn check_sumup_checkout(ctx: &Context, checkout_id: &str) -> MinilithResult<()> {
    let Some(txn) = sqlx::query_as!(
        TxnData,
        "select id, client_id, provider as \"provider: Provider\",
            sumup_checkout_id, sumup_reader_id
        from transactions
        where sumup_checkout_id = $1",
        checkout_id
    )
    .fetch_optional(&ctx.db)
    .await?
    else {
        // e.g. a checkout we created but whose transaction failed to insert
        return Ok(());
    };
    let _: ControlFlow<()> = fetch_transaction_info(ctx, txn).await?;
    Ok(())
}

/// # Errors
///
/// DB errors.
pub async fn initial_checks(ctx: &Arc<Context>) -> MinilithResult<()> {
    let unpaid_transactions = sqlx::query_as!(
        TxnData,
        "select id, provider as \"provider: Provider\", client_id,
            sumup_checkout_id, sumup_reader_id
        from transactions
        where payment_reference is null
        and cancelled_at is null"
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
    reconcile_refunds(ctx).await?;

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

    let refund_context = Arc::clone(ctx);
    tokio::spawn(async move {
        loop {
            drop(
                reconcile_refunds(&refund_context)
                    .await
                    .wrap_err_internal("l2: error from runtime->reconcile_refunds"),
            );

            tokio::time::sleep(std::time::Duration::from_mins(5)).await;
        }
    });
}

/// Settles refund claims (`refund_id`) that were written but never resolved: a `Swish` refund
/// whose callback never arrived, or a `SumUp` refund where we crashed between the call and the
/// write. Idempotent, so every instance may run it.
///
/// # Errors
///
/// DB errors.
pub async fn reconcile_refunds(ctx: &Context) -> MinilithResult<()> {
    let claims = sqlx::query!(
        "select id, refund_id as \"refund_id!\", provider as \"provider: Provider\"
        from transactions
        where refund_id is not null
        and refund_at is null
        and refund_reference is null
        order by created desc
        limit 500"
    )
    .fetch_all(&ctx.db)
    .await?;

    for claim in claims {
        let reconciled = match claim.provider {
            Provider::Swish => reconcile_swish_refund(ctx, claim.refund_id).await,
            Provider::Sumup => reconcile_sumup_refund(ctx, claim.id).await,
            Provider::Stripe | Provider::Free => Ok(()),
        };
        // failures are logged (and alerted when they are not a provider rejection); keep going
        drop(reconciled.wrap_err_internal("l2: refund reconciliation failed"));
    }
    Ok(())
}
/// Fetches and applies the state of the `Swish` refund instruction `refund_id`.
///
/// Called by `/swish-refund-callback` and by the runtime sweep, which picks up claims whose
/// callback never arrived (or raced the claim). Idempotent: only the caller that flips the
/// transaction to refunded sends the `Refunded` callback.
///
/// # Errors
///
/// DB and `Swish` transport errors.
pub(crate) async fn reconcile_swish_refund(ctx: &Context, refund_id: Uuid) -> MinilithResult<()> {
    // Read, call `Swish`, then write: never hold a connection across the HTTP request.
    let mut txn = ctx.db.begin().await?;
    let Some(transaction) = sqlx::query!(
        "select id, client_id, callback_url_v1
        from transactions where refund_id = $1",
        refund_id
    )
    .fetch_optional(&mut txn.executor())
    .await?
    else {
        return Ok(());
    };
    txn.commit().await?;

    let client = ctx.get_swish_client(&transaction.client_id).await?;
    let resp = client
        .get(swish::refund_url(refund_id, ctx.debug.enabled))
        .send()
        .await
        .wrap_err_internal("swish refund status transport error")?;
    drop(client);
    if !resp.status().is_success() {
        return Err(MinilithEndpointError::internal_error(
            "swish refund status fetch failed",
            (refund_id, resp.status()),
        ));
    }
    let refund = resp
        .json::<swish::Refund>()
        .await
        .wrap_err_internal("swish refund status parse failed")?;
    match refund.status {
        Some(swish::RefundStatus::Paid) => {
            let mut txn = ctx.db.begin().await?;
            let updated = sqlx::query!(
                "update transactions
                set refund_reference = $2, refund_at = now()
                where id = $1 and refund_at is null",
                transaction.id,
                refund
                    .payment_reference
                    .clone()
                    .unwrap_or_else(|| refund.id.to_string()),
            )
            .execute(&mut txn.executor())
            .await?;
            txn.commit().await?;
            if updated.rows_affected() == 1 {
                callback::send_refunded(
                    ctx,
                    transaction.id,
                    &transaction.callback_url_v1,
                    &transaction.client_id,
                )
                .await;
            }
        }
        Some(swish::RefundStatus::Error) => {
            alert(
                AlertLevel::L1,
                "l1: swish refund failed; the customer has not been refunded",
            );
            clear_refund_claim(ctx, transaction.id).await?;
        }
        Some(swish::RefundStatus::Created | swish::RefundStatus::Debited) | None => {}
    }
    Ok(())
}

/// Settles a `SumUp` refund claim left behind by a crash between the refund call and the write.
///
/// `SumUp` refunds are synchronous, so the only unknown is whether the refund reached them.
///
/// # Errors
///
/// DB and `SumUp` API errors.
pub(crate) async fn reconcile_sumup_refund(ctx: &Context, id: Uuid) -> MinilithResult<()> {
    let mut txn = ctx.db.begin().await?;
    let Some(transaction) = sqlx::query!(
        "select payment_reference, callback_url_v1, client_id
        from transactions
        where id = $1 and refund_id is not null and refund_at is null",
        id
    )
    .fetch_optional(&mut txn.executor())
    .await?
    else {
        return Ok(());
    };
    txn.commit().await?;
    let Some(payment_reference) = transaction.payment_reference else {
        return Ok(());
    };

    match sumup::refund_state(ctx, &transaction.client_id, &payment_reference).await? {
        sumup::RefundState::Refunded => {
            let mut txn = ctx.db.begin().await?;
            let updated = sqlx::query!(
                "update transactions
                set refund_reference = $2, refund_at = now(), refund_id = null
                where id = $1 and refund_at is null",
                id,
                payment_reference
            )
            .execute(&mut txn.executor())
            .await?;
            txn.commit().await?;
            if updated.rows_affected() == 1 {
                callback::send_refunded(
                    ctx,
                    id,
                    &transaction.callback_url_v1,
                    &transaction.client_id,
                )
                .await;
            }
        }
        sumup::RefundState::Failed => {
            alert(
                AlertLevel::L1,
                "l1: sumup refund failed; the customer has not been refunded",
            );
            clear_refund_claim(ctx, id).await?;
        }
        sumup::RefundState::NotRefunded => {
            // The refund never reached SumUp (we crashed before the call completed): release the
            // claim so the client can retry.
            clear_refund_claim(ctx, id).await?;
        }
    }
    Ok(())
}

/// # Errors
///
/// DB errors.
async fn clear_refund_claim(ctx: &Context, id: Uuid) -> MinilithResult<()> {
    let mut txn = ctx.db.begin().await?;
    sqlx::query!("update transactions set refund_id = null where id = $1", id)
        .execute(&mut txn.executor())
        .await?;
    txn.commit().await?;
    Ok(())
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
        and cancelled_at is null
        for update"
    )
    .fetch_all(&mut txn.executor())
    .await?;

    let mut cancelled_uuids = Vec::new();
    let mut cancelled = Vec::new();

    for transaction in timed_out_transactions {
        // holds a DB connection over HTTP requests, though this is fine since it's max one per
        // instance:)
        //
        // should probably be fixed though TODO
        if ctx.cancel_transaction(&transaction).await.is_ok() {
            cancelled_uuids.push(transaction.id);
            cancelled.push(transaction);
        } else {
            let txn = sqlx::query_as!(
                TxnData,
                "select id, client_id, provider as \"provider: Provider\",
                sumup_checkout_id, sumup_reader_id
                from transactions
                where id = $1",
                transaction.id
            )
            .fetch_one(&mut txn.executor())
            .await?;
            let _: ControlFlow<()> = fetch_transaction_info(ctx, txn).await?;
        }
    }

    sqlx::query!(
        "update transactions set cancelled_at = now()
        where cancelled_at is null
        and id = any($1)",
        &cancelled_uuids
    )
    .execute(&mut txn.executor())
    .await?;

    txn.commit().await?;

    callback::send_callbacks(
        ctx,
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

    Ok(())
}
