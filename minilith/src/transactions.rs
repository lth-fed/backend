//! Copied from `transactions/src/api.rs`.

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
    /// When this payment request will be cancelled.
    /// Will try to cancel within 30s.
    pub timeout: String,
    /// The list of items to be bought.
    pub wares: Vec<Ware>,
    /// Used for tracking cards in e.g. Stripe.
    pub customer_id: Option<String>,
    /// Redirected back when user completes transaction.
    pub stripe_success_url: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePaymentResponseFree {
    pub transaction_id: Uuid,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePaymentResponseSwish {
    /// See <https://developer.swish.nu/api/payment-request/v2#create-payment-request>.
    pub payment_request_token: String,
    pub transaction_id: Uuid,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePaymentResponseStripe {
    /// See <https://docs.stripe.com/api/checkout/sessions/object#checkout_session_object-url>.
    pub redirect_url: String,
    pub transaction_id: Uuid,
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
#[derive(Debug, Deserialize, Clone)]
pub struct SingleInfoResponse {
    // id: Uuid,
    // status: TransactionState,
    // customer_id: Option<String>,
    pub total_fees: i64,
    // provider: Provider,
    // payment_reference: Option<String>,
    // refund_reference: Option<String>,
}
