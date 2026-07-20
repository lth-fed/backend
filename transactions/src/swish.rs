use poem_openapi::{Enum, Object};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[must_use]
pub fn uuid_to_string(uuid: Uuid) -> String {
    let mut buffer = Uuid::encode_buffer();
    uuid.simple().encode_upper(&mut buffer).to_owned()
}

#[must_use]
pub fn payment_request_url(instruction_uuid: Uuid) -> String {
    let mut buffer = Uuid::encode_buffer();
    let uuid = instruction_uuid.simple().encode_upper(&mut buffer);
    #[cfg(debug_assertions)]
    {
        format!("https://mss.cpc.getswish.net/swish-cpcapi/api/v2/paymentrequests/{uuid}")
    }
    #[cfg(not(debug_assertions))]
    {
        format!("https://cpc.getswish.net/swish-cpcapi/api/v2/paymentrequests/{uuid}")
    }
}

// types:

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreatePaymentRequest {
    /// Target swish number.
    pub payee_alias: String,
    pub amount: String,
    pub currency: String,
    pub callback_url: String,
    pub message: String,
    pub callback_identifier: String,
}

#[derive(Enum, Clone, Copy, Debug, Deserialize)]
#[oai(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Paid,
    Declined,
    Error,
    Cancelled,
}

/// <https://developer.swish.nu/documentation/guides/create-a-payment-request#if-the-payment-is-successful>.
#[derive(Object, Debug, Clone, Deserialize)]
#[oai(rename_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub struct Callback {
    pub id: Uuid,
    // pub payee_payment_reference: Option<String>,
    /// Available if `status == Status::Paid`.
    pub payment_reference: Option<String>,
    // pub callback_url: String,
    // pub payer_alias: String,
    // pub payee_alias: String,
    // pub amount: f64,
    // pub currency: String,
    // pub message: String,
    pub status: Status,
    // pub date_created: String,
    // pub date_paid: String,
    // pub error_code: Option<serde_json::Value>,
    pub error_message: Option<String>,
}

#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub enum PaymentRequestPatchOperation {
    Replace,
}
#[derive(Serialize, Clone, Debug)]
pub struct PaymentRequestPatch {
    pub op: PaymentRequestPatchOperation,
    pub path: String,
    pub value: String,
}
