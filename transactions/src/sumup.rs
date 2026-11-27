//! `SumUp` card payments and readers, on top of the official `sumup` SDK.

use minilith_errors::{MinilithEndpointError, MinilithErrorOptionExt as _, MinilithResult};
use poem_openapi::Object;
use sumup::Currency;
use sumup::resources::checkouts::HostedCheckout;
use sumup::resources::readers::{
    CreateCheckoutRequestTotalAmount, CreateReaderCheckoutResponse, GetReaderCheckoutResponse,
    ReaderDeviceModel, ReaderStatus,
};
use sumup::resources::transactions::GetParams;

use crate::context::Context;

pub(crate) const PRODUCTION: &str = "https://api.sumup.com";
pub(crate) const SANDBOX: &str = "https://api.sumup.com";

fn map_err<E: std::fmt::Debug>(error: &sumup::SdkError<E>) -> MinilithEndpointError {
    let status = error.status();
    if status.is_some_and(|status| status.is_client_error()) {
        return MinilithEndpointError::bad_frontend_code(
            "SumUp rejected the request",
            error.to_string(),
        );
    }
    MinilithEndpointError::internal_error("SumUp API request failed", (status, error.to_string()))
}

#[derive(Object, Debug, Clone)]
pub struct Reader {
    pub id: String,
    pub name: String,
    pub status: String,
    pub device: Option<ReaderDevice>,
}
#[derive(Object, Debug, Clone)]
pub struct ReaderDevice {
    pub identifier: String,
    pub model: String,
}
fn map_reader(reader: sumup::resources::readers::Reader) -> Reader {
    let status = match reader.status {
        ReaderStatus::Unknown => "unknown",
        ReaderStatus::Processing => "processing",
        ReaderStatus::Paired => "paired",
        ReaderStatus::Expired => "expired",
        ReaderStatus::Other(ref other) => other,
    };
    let device = ReaderDevice {
        identifier: reader.device.identifier,
        model: match reader.device.model {
            ReaderDeviceModel::Solo => "solo".to_owned(),
            ReaderDeviceModel::VirtualSolo => "virtual-solo".to_owned(),
            ReaderDeviceModel::Other(other) => other,
        },
    };
    Reader {
        id: reader.id,
        name: reader.name,
        status: status.to_owned(),
        device: Some(device),
    }
}

#[derive(Debug, Clone)]
pub struct Checkout {
    pub id: String,
    pub status: CheckoutStatus,
    pub hosted_checkout_url: Option<String>,
    successful_transaction_id: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutStatus {
    Pending,
    Failed,
    Paid,
    Expired,
}
impl Checkout {
    /// The id of the successful payment, used for refunds.
    #[must_use]
    pub fn transaction_id(&self) -> Option<&str> {
        self.successful_transaction_id.as_deref()
    }
}
fn map_checkout_status(
    status: Option<&sumup::resources::checkouts::CheckoutSuccessStatus>,
) -> CheckoutStatus {
    use sumup::resources::checkouts::CheckoutSuccessStatus as Sdk;
    match status {
        Some(Sdk::Paid) => CheckoutStatus::Paid,
        Some(Sdk::Failed) => CheckoutStatus::Failed,
        Some(Sdk::Expired) => CheckoutStatus::Expired,
        Some(Sdk::Pending | Sdk::Other(_)) | None => CheckoutStatus::Pending,
    }
}

/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn create_checkout(
    ctx: &Context,
    client_id: &str,
    transaction_id: uuid::Uuid,
    amount: i64,
    description: &str,
    redirect_url: Option<&str>,
) -> MinilithResult<Checkout> {
    let client = ctx.get_sumup_client(client_id).await?;
    #[allow(
        clippy::cast_precision_loss,
        reason = "ören are far below f32's exact integer range"
    )]
    let amount = amount as f32 / 100.0;

    let request = sumup::resources::checkouts::CreateRequest::builder()
        .checkout_reference(transaction_id.to_string())
        .amount(amount)
        .currency(Currency::SEK)
        .merchant_code(client.merchant_code.clone())
        .description(description)
        .return_url(format!("{}/v0/sumup-callback", crate::api::DOMAIN))
        .hosted_checkout(HostedCheckout::builder().enabled(true).build());
    let request = if let Some(redirect_url) = redirect_url {
        request.redirect_url(redirect_url).build()
    } else {
        request.build()
    };

    let checkout = client
        .checkouts()
        .create(request)
        .await
        .map_err(|error| map_err(&error))?;
    // The create response does not carry the payment transaction; fetch it when we need it.
    Ok(Checkout {
        id: checkout.id.unwrap_or_default(),
        status: match checkout.status {
            Some(sumup::resources::checkouts::CheckoutStatus::Paid) => CheckoutStatus::Paid,
            Some(sumup::resources::checkouts::CheckoutStatus::Failed) => CheckoutStatus::Failed,
            Some(sumup::resources::checkouts::CheckoutStatus::Expired) => CheckoutStatus::Expired,
            Some(
                sumup::resources::checkouts::CheckoutStatus::Pending
                | sumup::resources::checkouts::CheckoutStatus::Other(_),
            )
            | None => CheckoutStatus::Pending,
        },
        hosted_checkout_url: checkout.hosted_checkout_url,
        successful_transaction_id: checkout
            .transactions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|transaction| transaction.status == Some(sumup::TransactionStatus::Successful))
            .and_then(|transaction| transaction.id.clone()),
    })
}

/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn get_checkout(
    ctx: &Context,
    client_id: &str,
    checkout_id: &str,
) -> MinilithResult<Checkout> {
    let client = ctx.get_sumup_client(client_id).await?;
    let checkout = client
        .checkouts()
        .get(checkout_id)
        .await
        .map_err(|error| map_err(&error))?;
    Ok(Checkout {
        id: checkout.id.unwrap_or_else(|| checkout_id.to_owned()),
        status: map_checkout_status(checkout.status.as_ref()),
        hosted_checkout_url: checkout.hosted_checkout_url,
        successful_transaction_id: checkout
            .transactions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|transaction| transaction.status == Some(sumup::TransactionStatus::Successful))
            .and_then(|transaction| transaction.id.clone()),
    })
}

/// Cancels a pending checkout. Returns `Ok(false)` when `SumUp` says it can no longer be
/// cancelled (it was paid or already expired in the meantime).
///
/// # Errors
///
/// `SumUp` transport errors.
pub(crate) async fn deactivate_checkout(
    ctx: &Context,
    client_id: &str,
    checkout_id: &str,
) -> MinilithResult<bool> {
    let client = ctx.get_sumup_client(client_id).await?;
    match client.checkouts().deactivate(checkout_id).await {
        Ok(_) => Ok(true),
        Err(error) => {
            // `SumUp` rejects cancelling checkouts that are already paid or expired. Either way
            // the caller re-reads the checkout, so there is nothing to recover from here — only
            // report it (4xx: expected, otherwise: alert) and treat it as "not cancelled".
            drop(map_err(&error));
            Ok(false)
        }
    }
}

/// Refunds (part of) a transaction. `None` refunds everything.
///
/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn refund_transaction(
    ctx: &Context,
    client_id: &str,
    transaction_id: &str,
    amount: Option<i64>,
) -> MinilithResult<()> {
    let client = ctx.get_sumup_client(client_id).await?;
    let body = amount.map(|amount| sumup::resources::transactions::RefundRequest {
        amount: Some(amount),
    });
    client
        .transactions()
        .refund(&client.merchant_code, transaction_id, body)
        .await
        .map_err(|error| map_err(&error))?;
    Ok(())
}

/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn list_readers(ctx: &Context, client_id: &str) -> MinilithResult<Vec<Reader>> {
    let client = ctx.get_sumup_client(client_id).await?;
    let response = client
        .readers()
        .list(&client.merchant_code)
        .await
        .map_err(|error| map_err(&error))?;
    Ok(response.items.into_iter().map(map_reader).collect())
}

/// # Errors
///
/// `SumUp` API errors (e.g. `404` when the pairing code has expired).
pub(crate) async fn create_reader(
    ctx: &Context,
    client_id: &str,
    pairing_code: &str,
    name: &str,
) -> MinilithResult<Reader> {
    let client = ctx.get_sumup_client(client_id).await?;
    let reader = client
        .readers()
        .create(
            &client.merchant_code,
            sumup::resources::readers::CreateRequest::builder()
                .pairing_code(pairing_code)
                .name(name)
                .build(),
        )
        .await
        .map_err(|error| map_err(&error))?;
    Ok(map_reader(reader))
}

/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn update_reader(
    ctx: &Context,
    client_id: &str,
    reader_id: &str,
    name: Option<&str>,
) -> MinilithResult<Reader> {
    let client = ctx.get_sumup_client(client_id).await?;
    let request = if let Some(name) = name {
        sumup::resources::readers::UpdateRequest::builder()
            .name(name)
            .build()
    } else {
        sumup::resources::readers::UpdateRequest::builder().build()
    };
    let reader = client
        .readers()
        .update(&client.merchant_code, reader_id, request)
        .await
        .map_err(|error| map_err(&error))?;
    Ok(map_reader(reader))
}

/// Deletes a reader from the merchant account. A reader that is already gone counts as deleted.
///
/// # Errors
///
/// `SumUp` API errors other than `404`.
pub(crate) async fn delete_reader(
    ctx: &Context,
    client_id: &str,
    reader_id: &str,
) -> MinilithResult<()> {
    let client = ctx.get_sumup_client(client_id).await?;
    match client
        .readers()
        .delete(&client.merchant_code, reader_id)
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.status() == Some(reqwest::StatusCode::NOT_FOUND) => Ok(()),
        Err(error) => Err(map_err(&error)),
    }
}

/// Starts a payment on a physical reader. Returns `(checkout_id, client_transaction_id)`.
///
/// # Errors
///
/// `SumUp` API errors, e.g. when the reader is offline.
pub(crate) async fn create_reader_checkout(
    ctx: &Context,
    client_id: &str,
    reader_id: &str,
    amount: i64,
    description: &str,
) -> MinilithResult<(String, String)> {
    let client = ctx.get_sumup_client(client_id).await?;
    let request = sumup::resources::readers::CreateCheckoutRequest::builder()
        .total_amount(
            CreateCheckoutRequestTotalAmount::builder()
                .currency("SEK")
                .minor_unit(2)
                .value(amount)
                .build(),
        )
        .description(description)
        .return_url(format!("{}/v0/sumup-callback", crate::api::DOMAIN))
        .build();
    let CreateReaderCheckoutResponse { data } = client
        .readers()
        .create_checkout(&client.merchant_code, reader_id, request)
        .await
        .map_err(|error| map_err(&error))?;
    let checkout_id = data
        .checkout_id
        .unwrap_or_else(|| data.client_transaction_id.clone());
    Ok((checkout_id, data.client_transaction_id))
}

/// `(status, client_transaction_id)` of a reader checkout.
///
/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn get_reader_checkout(
    ctx: &Context,
    client_id: &str,
    reader_id: &str,
    checkout_id: &str,
) -> MinilithResult<(CheckoutStatus, String)> {
    use sumup::resources::readers::GetReaderCheckoutResponseDataStatus as Sdk;
    let client = ctx.get_sumup_client(client_id).await?;
    let GetReaderCheckoutResponse { data } = client
        .readers()
        .get_checkout(&client.merchant_code, reader_id, checkout_id)
        .await
        .map_err(|error| map_err(&error))?;
    let status = match data.payment_status.as_str() {
        "successful" => CheckoutStatus::Paid,
        "failed" | "cancelled" => CheckoutStatus::Failed,
        _ => match data.status {
            Sdk::Successful => CheckoutStatus::Paid,
            Sdk::Failed | Sdk::Cancelled => CheckoutStatus::Failed,
            Sdk::Pending | Sdk::Other(_) => CheckoutStatus::Pending,
        },
    };
    Ok((status, data.client_transaction_id))
}

/// What `SumUp` knows about refunds on a payment transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefundState {
    /// Refunded in full or in part.
    Refunded,
    /// A refund attempt did not complete.
    Failed,
    /// No refund has been made.
    NotRefunded,
}

/// Used by the runtime sweep to settle refund claims left behind by a crash.
///
/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn refund_state(
    ctx: &Context,
    client_id: &str,
    transaction_id: &str,
) -> MinilithResult<RefundState> {
    use sumup::resources::transactions::TransactionFullSimpleStatus as Sdk;
    let client = ctx.get_sumup_client(client_id).await?;
    let transaction = client
        .transactions()
        .get(
            &client.merchant_code,
            GetParams::builder().id(transaction_id).build(),
        )
        .await
        .map_err(|error| map_err(&error))?;
    Ok(match transaction.simple_status {
        Some(Sdk::Refunded) => RefundState::Refunded,
        Some(Sdk::RefundFailed) => RefundState::Failed,
        _ => RefundState::NotRefunded,
    })
}

/// Resolves the `SumUp` transaction id for a `client_transaction_id` (reader payments).
///
/// # Errors
///
/// `SumUp` API errors.
pub(crate) async fn transaction_id_for_client_transaction(
    ctx: &Context,
    client_id: &str,
    client_transaction_id: &str,
) -> MinilithResult<String> {
    let client = ctx.get_sumup_client(client_id).await?;
    let transaction = client
        .transactions()
        .get(
            &client.merchant_code,
            GetParams::builder()
                .client_transaction_id(client_transaction_id)
                .build(),
        )
        .await
        .map_err(|error| map_err(&error))?;
    transaction
        .id
        .wrap_err_internal("sumup: transaction has no id")
}
