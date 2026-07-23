//! Copied from `transactions/src/api.rs`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
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
    pub timeout: OffsetDateTime,
    /// The list of items to be bought.
    pub wares: Vec<Ware>,
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
