use std::ops::Deref;

use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _};
use poem_openapi::{ApiExtractor, Enum, Object};
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::{AuthUrl, JwkContext, TransactionsUrl};

macro_rules! impl_api_extractor {
    ($type: ty, $ctx: ty) => {
        impl<'a> ApiExtractor<'a> for $type {
            // to make the OpenApi look pretty
            // inspired by the impl of PlainText<String>
            type ParamType = ();
            type ParamRawType = ();

            const TYPES: &'static [poem_openapi::ApiExtractorType] =
                &[poem_openapi::ApiExtractorType::RequestObject];
            const PARAM_IS_REQUIRED: bool = false;

            fn request_meta() -> Option<poem_openapi::registry::MetaRequest> {
                let mut jwt = poem_openapi::registry::MetaSchema::new("string");
                jwt.example = Some(
                    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
                    eyJsb2dnZWRJbkFzIjoiYWRtaW4iLCJpYXQiOjE0MjI3Nzk2Mzh9.\
                    gzSraSYS8EXBxLN_oWnFSRgCzcmJmMjLiuyu5CSpyHI"
                        .into(),
                );

                Some(poem_openapi::registry::MetaRequest {
                    description: None,
                    content: vec![poem_openapi::registry::MetaMediaType {
                        content_type: "application/jwt",
                        schema: poem_openapi::registry::MetaSchemaRef::Inline(Box::new(jwt)),
                    }],
                    required: true,
                })
            }

            async fn from_request(
                request: &'a poem::Request,
                body: &mut poem::RequestBody,
                _param_opts: poem_openapi::ExtractParamOptions<()>,
            ) -> poem::Result<Self> {
                let context: &$ctx = request.data().ok_or_else(|| {
                    MinilithEndpointError::internal_error("AuthContext not registered as data!")
                })?;
                let body = body.take().map_err(|err| {
                    error!(?err, "Somebody took our body!");
                    MinilithEndpointError::internal_error("")
                })?;
                let body = body
                    .into_string()
                    .await
                    .wrap_err_bad_user("invalid utf8 body", "<body>")?;
                let data: $type = crate::decode_jwt(&body, &context)
                    .wrap_err_unauthorized("jwt invalid on decode")?
                    .claims;
                Ok(data)
            }
        }
    };
}

/// [`JwkContext<AuthUrl>`] MUST BE registered using [`poem::EndpointExt::data`].
///
/// # Example
///
/// ```no_compile
/// # use poem_openapi::OpenApi;
/// # use fed_auth_verifier::AuthCallbackDataV1;
/// pub struct Context {
///     db: sqlx::PgPool,
/// }
/// pub struct Router {
///     context: Context,
/// }
/// #[OpenApi]
/// impl Router {
///     #[oai(path = "/callback/v1", method = "post")]
///     async fn callback(&self, data: AuthCallbackDataV1) {
///         sqlx::query!(
///             "insert into users values ($1, $2, $3)",
///             data.sub, data.full_name, data.email,
///         )
///         .execute(&self.context.db)
///         .await
///         .unwrap();
///     }
/// }
/// ```
#[derive(Debug, Object, Deserialize)]
pub struct AuthCallbackDataV1 {
    pub sub: String,
    pub email: String,
    pub full_name: String,
}
impl_api_extractor!(AuthCallbackDataV1, JwkContext<AuthUrl>);

#[derive(Serialize, Deserialize, Debug, Enum, Clone, Copy)]
pub enum TransactionState {
    Pending,
    Paid,
    Refunded,
    /// When getting the status of a transaction, this may indicate the transaction didn't ever
    /// exist.
    Cancelled,
}
#[derive(Serialize, Deserialize, Debug, Object, Clone, Copy)]
pub struct TransactionInfo {
    pub state: TransactionState,
}
#[derive(Serialize, Deserialize, Debug, Object, Clone, Copy)]
pub struct TransactionCallbackInfo {
    pub transaction_id: Uuid,
    #[serde(flatten)]
    #[oai(flatten)]
    pub inner: TransactionInfo,
}

/// [`JwkContext<TransactionsUrl>`] MUST BE registered using [`poem::EndpointExt::data`].
///
/// # Example
///
/// ```no_compile
/// # use poem_openapi::OpenApi;
/// # use fed_auth_verifier::TransactionsCallbackDataV1;
/// pub struct Context {
///     db: sqlx::PgPool,
/// }
/// pub struct Router {
///     context: Context,
/// }
/// #[OpenApi]
/// impl Router {
///     #[oai(path = "/callback/v1", method = "post")]
///     async fn callback(&self, data: TransactionsCallbackDataV1) {
///         sqlx::query!(
///             "update transactions set status = $2 where id = $1",
///             data.id, data.status,
///         )
///         .execute(&self.context.db)
///         .await
///         .unwrap();
///     }
/// }
/// ```
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
#[must_use]
pub struct TransactionsCallbackDataV1(pub Vec<TransactionCallbackInfo>);
impl TransactionsCallbackDataV1 {
    pub fn single(info: TransactionCallbackInfo) -> Self {
        Self(vec![info])
    }
}
impl Deref for TransactionsCallbackDataV1 {
    type Target = Vec<TransactionCallbackInfo>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl_api_extractor!(TransactionsCallbackDataV1, JwkContext<TransactionsUrl>);
