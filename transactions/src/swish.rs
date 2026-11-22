use poem_openapi::{Enum, Object};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[must_use]
pub fn uuid_to_string(uuid: Uuid) -> String {
    let mut buffer = Uuid::encode_buffer();
    uuid.simple().encode_upper(&mut buffer).to_owned()
}

#[derive(Clone, Copy, Debug)]
pub enum ApiVersion {
    V1,
    V2,
}

/// The refund is created with `PUT`, at this url.
#[must_use]
pub fn refund_instruction_url(instruction_uuid: Uuid, debug: bool) -> String {
    let mut buffer = Uuid::encode_buffer();
    let uuid = instruction_uuid.simple().encode_upper(&mut buffer);
    if debug {
        format!("https://mss.cpc.getswish.net/swish-cpcapi/api/v2/refunds/{uuid}")
    } else {
        format!("https://cpc.getswish.net/swish-cpcapi/api/v2/refunds/{uuid}")
    }
}
/// Retrieve the status of a refund.
#[must_use]
pub fn refund_url(instruction_uuid: Uuid, debug: bool) -> String {
    let mut buffer = Uuid::encode_buffer();
    let uuid = instruction_uuid.simple().encode_upper(&mut buffer);
    if debug {
        format!("https://mss.cpc.getswish.net/swish-cpcapi/api/v1/refunds/{uuid}")
    } else {
        format!("https://cpc.getswish.net/swish-cpcapi/api/v1/refunds/{uuid}")
    }
}

#[must_use]
pub fn payment_request_url(version: ApiVersion, instruction_uuid: Uuid, debug: bool) -> String {
    let mut buffer = Uuid::encode_buffer();
    let uuid = instruction_uuid.simple().encode_upper(&mut buffer);
    let v_string = match version {
        ApiVersion::V1 => "v1",
        ApiVersion::V2 => "v2",
    };
    if debug {
        format!("https://mss.cpc.getswish.net/swish-cpcapi/api/{v_string}/paymentrequests/{uuid}")
    } else {
        format!("https://cpc.getswish.net/swish-cpcapi/api/{v_string}/paymentrequests/{uuid}")
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

#[derive(Enum, Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[oai(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Paid,
    Declined,
    Error,
    Cancelled,
    /// Undocumented:( .
    Created,
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
    pub status: Option<Status>,
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

/// <https://developer.swish.nu/api/refunds>.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RefundRequest {
    /// `payment_reference` of the original payment.
    pub original_payment_reference: String,
    pub callback_url: String,
    /// The Swish number of the merchant making the refund.
    pub payer_alias: String,
    /// In SEK, e.g. `100.00`.
    pub amount: String,
    /// MUST be `SEK`.
    pub currency: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Refund {
    pub id: Uuid,
    pub status: Option<RefundStatus>,
    /// The refund's payment reference, available when it is done.
    pub payment_reference: Option<String>,
}

#[derive(Enum, Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[oai(rename_all = "UPPERCASE")]
#[serde(rename_all = "UPPERCASE")]
pub enum RefundStatus {
    Created,
    Debited,
    Paid,
    Error,
}
