//! Copied from `transactions/src/api.rs`.

use fed_auth_verifier::callbacks::TransactionState;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Default, Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    /// Swedish crowns.
    #[default]
    Sek,
}
#[derive(Debug, Clone, Serialize)]
pub struct Ware {
    /// When buying several of one item, append e.g. `x3` to the name and increase `amount`.
    /// This is flexible for e.g. sales when buying more than 1.
    pub name: String,
    /// The total amount (inclusive tax) for this ware. In ören.
    pub amount: i64,
    /// The tax rate. Must be `> 1` (e.g. `1.25` for common moms in Sweden).
    pub tax: f64,
    /// The currency in which this transactions is made.
    pub currency: Currency,
}
#[derive(Debug, Clone, Serialize)]
pub struct CreatePaymentRequest {
    /// From `/init`.
    pub id: Uuid,
    /// When this payment request will be cancelled.
    /// Will try to cancel within 30s.
    pub timeout: String,
    /// The list of items to be bought.
    pub wares: Vec<Ware>,
    /// Used for tracking cards in e.g. `SumUp`.
    pub customer_id: Option<String>,
    /// Redirected back when user completes transaction. Required for `SumUp`.
    pub redirect_url: Option<String>,
    /// Starts the payment on this `SumUp` reader instead of a hosted checkout.
    pub reader_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePaymentResponseSwish {
    /// See <https://developer.swish.nu/api/payment-request/v2#create-payment-request>.
    pub payment_request_token: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePaymentResponseSumup {
    /// `SumUp`-hosted payment page. `null` for physical sales on a reader.
    pub redirect_url: Option<String>,
}

#[derive(Serialize, Debug, Clone, Copy)]
pub enum Language {
    #[serde(rename = "sv")]
    Swedish,
    #[serde(rename = "en")]
    English,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReceiptRequest {
    pub language: Language,
    pub customer_name: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct InfoRequest {
    pub transaction_ids: Vec<Uuid>,
}
#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Swish,
    /// Legacy: kept so historical rows still decode.
    Stripe,
    Free,
    Sumup,
}
#[derive(Debug, Deserialize, Clone)]
pub struct SingleInfoResponse {
    pub id: Uuid,
    pub state: TransactionState,
    // customer_id: Option<String>,
    /// What the provider charged for this payment (in ören); refund fees are separate.
    pub purchase_fees: i64,
    /// Sum of the payment's wares (in ören): what the buyer paid.
    pub total: i64,
    /// What refunding this transaction costs (in ören): flat on `Swish`, free on `SumUp`.
    pub refund_fee: i64,
    /// The provider the transaction was made with.
    /// `null` when the transaction does not exist.
    pub provider: Option<Provider>,
    // payment_reference: Option<String>,
    // refund_reference: Option<String>,
}
