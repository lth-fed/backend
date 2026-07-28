use std::collections::HashMap;

use minilith_errors::{MinilithEndpointError, MinilithResult};
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use crate::context::Context;
use crate::{CallbackEvent, CallbackInfo, TransactionInfo, TransactionState, swish};

pub async fn send_callbacks(
    client: &reqwest::Client,
    signing_key: &jsonwebtoken::EncodingKey,
    events: impl Iterator<Item = CallbackEvent>,
) {
    let mut endpoints: HashMap<String, (String, Vec<CallbackInfo>)> = HashMap::new();
    for event in events {
        let entry = endpoints.entry(event.callback_url_v1.clone());
        entry
            .or_insert_with(|| (event.client_id.clone(), Vec::new()))
            .1
            .push(event.inner);
    }
    for (endpoint, (client_id, infos)) in endpoints {
        #[derive(Serialize)]
        pub struct StandardClaims {
            pub exp: u64,
            pub iat: u64,
            pub nbf: u64,
            pub aud: String,
            pub events: Vec<CallbackInfo>,
        }

        let now = jsonwebtoken::get_current_timestamp();
        let claims = StandardClaims {
            exp: now + 60,
            iat: now,
            nbf: now,
            aud: client_id,
            events: infos,
        };

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
        header.kid = Some("main".to_owned());
        let Ok(token) = jsonwebtoken::encode(&header, &claims, signing_key) else {
            continue;
        };

        match client
            .post(&endpoint)
            .body(token)
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
        "select id, callback_identifier, callback_url_v1, client_id
        from transactions where id = $1",
        data.id
    )
    .fetch_one(&ctx.db)
    .await?;

    if validate_callback_identifier
        .is_some_and(|callback_identifier| callback_identifier != transaction.callback_identifier)
    {
        return Err(MinilithEndpointError::unauthorized(
            "callbackIdentifier not valid",
            "",
        ));
    }
    match data.status {
        None => {}
        Some(swish::Status::Paid) if let Some(payment_reference) = data.payment_reference => {
            send_callbacks(
                &ctx.client,
                &ctx.signing_key,
                [CallbackEvent {
                    callback_url_v1: transaction.callback_url_v1,
                    client_id: transaction.client_id,
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
            .await?;
        }
        Some(swish::Status::Paid) => {
            return Err(MinilithEndpointError::bad_frontend_code(
                "paymentReference has to be non-null when PAID.",
                "",
            ));
        }
        _ => {
            send_callbacks(
                &ctx.client,
                &ctx.signing_key,
                [CallbackEvent {
                    callback_url_v1: transaction.callback_url_v1,
                    client_id: transaction.client_id,
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
                .await?;
        }
    }
    Ok(())
}
