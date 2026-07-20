use std::collections::HashMap;

use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _, MinilithResult};
use tracing::warn;
use uuid::Uuid;

use crate::context::Context;
use crate::{CallbackEvent, CallbackInfo, TransactionInfo, TransactionState, swish};

pub async fn send_callbacks(client: &reqwest::Client, events: impl Iterator<Item = CallbackEvent>) {
    let mut endpoints: HashMap<String, Vec<CallbackInfo>> = HashMap::new();
    for event in events {
        let entry = endpoints.entry(event.callback_url_v1.clone());
        entry.or_default().push(event.inner);
    }
    for (endpoint, infos) in endpoints {
        match client
            .post(&endpoint)
            .json(&infos)
            .send()
            .await
            .map(reqwest::Response::error_for_status)
        {
            Err(err) => {
                warn!(?err, "Failed to send callback to {endpoint}");
            }
            Ok(Err(err)) => {
                warn!(?err, "Failed to send callback to {endpoint}, status bad");
            }
            Ok(Ok(_resp)) => {}
        }
    }
}

/// # Errors
///
/// DB errors & `callback_identifier` validation error.
pub async fn handle_callback_to_us(
    ctx: &Context,
    data: swish::Callback,
    validate_callback_identifier: Option<Uuid>,
) -> MinilithResult<()> {
    let transaction = sqlx::query!(
        "select id, callback_identifier, callback_url_v1 from transactions where id = $1",
        data.id
    )
    .fetch_one(&ctx.db)
    .await
    .wrap_err_db()?;

    if validate_callback_identifier
        .is_some_and(|callback_identifier| callback_identifier != transaction.callback_identifier)
    {
        return Err(MinilithEndpointError::unauthorized(
            "callbackIdentifier not valid",
            "",
        ));
    }
    match data.payment_reference {
        None => {
            send_callbacks(
                &ctx.client,
                [CallbackEvent {
                    callback_url_v1: transaction.callback_url_v1,
                    inner: CallbackInfo {
                        transaction_id: transaction.id,
                        inner: TransactionInfo {
                            state: TransactionState::Cancelled,
                        },
                    },
                }]
                .into_iter(),
            )
            .await;
            // it's important that we write after! If we are stopped or crash, we want to send
            // the callback, then when we start up again realize this is should be deleted and
            // send another callback.
            sqlx::query!("delete from transactions where id = $1", data.id)
                .execute(&ctx.db)
                .await
                .wrap_err_db()?;
        }
        Some(payment_reference) => {
            send_callbacks(
                &ctx.client,
                [CallbackEvent {
                    callback_url_v1: transaction.callback_url_v1,
                    inner: CallbackInfo {
                        transaction_id: transaction.id,
                        inner: TransactionInfo {
                            state: TransactionState::Paid,
                        },
                    },
                }]
                .into_iter(),
            )
            .await;
            sqlx::query!(
                "update transactions set payment_reference = $2 where id = $1",
                data.id,
                payment_reference
            )
            .execute(&ctx.db)
            .await
            .wrap_err_db()?;
        }
    }
    Ok(())
}
