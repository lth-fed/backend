use std::collections::HashMap;
use std::fmt::Display;

use fed_auth_verifier::AuthContext;
use poem::http::Method;
use poem::middleware::Cors;
use poem::{Endpoint, EndpointExt as _, Route};
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object, OpenApiService};
use sqlx::PgPool;
use tracing::error;

pub mod activities;
pub mod context;
pub mod group;
pub mod healthcheck;
pub mod ticket;
pub mod user;

pub use context::Context;

pub type DbInternationalizedString = sqlx::types::Json<InternationalizedString>;
#[derive(Debug, Clone, poem_openapi::NewType, serde::Serialize, serde::Deserialize)] // eventually implement Deserialize ourselves
#[oai(from_multipart = false, from_parameter = false, to_header = false)]
#[serde(transparent)]
pub struct InternationalizedString(HashMap<String, String>);
impl InternationalizedString {
    /// # Panics
    ///
    /// None.
    #[must_use]
    pub fn to_json_value(self) -> serde_json::Value {
        #[allow(clippy::expect_used, reason = "See string below")]
        serde_json::to_value(self.0)
            .expect("we know a hashmap will always serialize & we also know it has string keys")
    }
}
impl From<DbInternationalizedString> for InternationalizedString {
    fn from(value: DbInternationalizedString) -> Self {
        value.0
    }
}

pub type MinilithResult<T> = Result<T, MinilithEndpointError>;

#[derive(Enum, Debug, Clone, Copy)]
pub enum MinilithErrorKind {
    /// Code prefix: `BR`.
    BadFrontend,
    /// Code prefix: `BU`.
    BadUser,
    /// Code prefix: `AUTH`.
    Unauthorized,
    /// Code prefix: `NF`.
    NotFound,
    /// Code prefix: `DB`.
    Database,
    /// Code prefix: `ENC`.
    EncryptionDecryption,
    /// Code prefix: `UK`.
    Other,
}
impl MinilithErrorKind {
    pub fn to_full_code(&self, subcode: impl AsRef<str>) -> String {
        format!("{}_{}", self.as_code_prefix(), subcode.as_ref())
    }
    #[must_use]
    pub fn as_code_prefix(&self) -> &'static str {
        match self {
            MinilithErrorKind::BadFrontend => "BR",
            MinilithErrorKind::BadUser => "BU",
            MinilithErrorKind::Unauthorized => "AUTH",
            MinilithErrorKind::NotFound => "NF",
            MinilithErrorKind::Database => "DB",
            MinilithErrorKind::EncryptionDecryption => "ENC",
            MinilithErrorKind::Other => "UK",
        }
    }
}
#[derive(Object, Debug)]
#[must_use]
pub struct MinilithError {
    code: String,
    kind: MinilithErrorKind,
    message: String,
    field: Option<String>,
}
impl MinilithError {
    /// You should most probably [`error!`] also.
    pub fn internal_inconsistency(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            kind: MinilithErrorKind::EncryptionDecryption,
            message: "internal unrecoverable error".into(),
            field: None,
        }
    }
    pub fn not_found() -> Self {
        Self {
            code: "NF".into(),
            kind: MinilithErrorKind::NotFound,
            message: String::new(),
            field: None,
        }
    }
}

#[derive(ApiResponse, Debug)]
#[oai(bad_request_handler = "bad_request_handler")]
#[must_use]
pub enum MinilithEndpointError {
    #[oai(status = 400)]
    BadRequest(Json<MinilithError>),
    #[oai(status = 401)]
    Unauthorized(Json<MinilithError>),
    #[oai(status = 404)]
    NotFound(Json<MinilithError>),
    #[oai(status = 500)]
    InternalServerError(Json<MinilithError>),
}
impl MinilithEndpointError {
    pub fn bad_frontend_code(subcode: impl AsRef<str>, message: impl Into<String>) -> Self {
        Self::BadRequest(Json(MinilithError {
            code: MinilithErrorKind::BadFrontend.to_full_code(subcode),
            kind: MinilithErrorKind::BadFrontend,
            message: message.into(),
            field: None,
        }))
    }
    pub fn bad_user_input(
        subcode: impl AsRef<str>,
        message: impl Into<String>,
        field: impl Into<String>,
    ) -> Self {
        Self::BadRequest(Json(MinilithError {
            code: MinilithErrorKind::BadUser.to_full_code(subcode),
            kind: MinilithErrorKind::BadUser,
            message: message.into(),
            field: Some(field.into()),
        }))
    }
    /// Only to be used for actual auth problems, not for when access is limited.
    pub fn unauthorized(subcode: impl AsRef<str>, message: impl Into<String>) -> Self {
        Self::Unauthorized(Json(MinilithError {
            code: MinilithErrorKind::Unauthorized.to_full_code(subcode),
            kind: MinilithErrorKind::Unauthorized,
            message: message.into(),
            field: None,
        }))
    }
    pub fn not_found() -> Self {
        Self::NotFound(Json(MinilithError {
            code: MinilithErrorKind::NotFound.as_code_prefix().into(),
            kind: MinilithErrorKind::NotFound,
            message: "resource not found, try reloading app".into(),
            field: None,
        }))
    }
    /// Notice that it's the FULL code, not just subcode!
    pub fn internal_error(
        subcode: impl AsRef<str>,
        kind: MinilithErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self::InternalServerError(Json(MinilithError {
            code: kind.to_full_code(subcode),
            kind,
            message: message.into(),
            field: None,
        }))
    }
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "poem requires us to consume it"
)]
fn bad_request_handler(err: poem::Error) -> MinilithEndpointError {
    // the errors we get from poem are when it's parsing the parameters (i think!)
    MinilithEndpointError::bad_frontend_code(
        "PARAM",
        format!("Something went wrong! We received unexpected data: {err}"),
    )
}
impl MinilithEndpointError {}
pub trait MinilithErrorResultExt {
    type OkType;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_db`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Leaks the [`Display`] impl of the error to the client.
    fn wrap_err_bad_request(self, subcode: impl AsRef<str>) -> MinilithResult<Self::OkType>;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_db`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_form(
        self,
        subcode: impl AsRef<str>,
        field: impl Into<String>,
    ) -> MinilithResult<Self::OkType>;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_db`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_unauthorized(self, subcode: impl AsRef<str>) -> MinilithResult<Self::OkType>;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_db`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_not_found(self) -> MinilithResult<Self::OkType>;
    /// Subcode should be globally unique. A common way to do this is to describe what it does but
    /// very shortened. Say we're wrapping this in the activities list handler, subcode could be
    /// `ACT_USER_LIST`.
    ///
    /// Subcode is called subcode and not code because a prefix is added to make it a complete code
    /// (`DB_` in this case).
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_db(self, subcode: impl AsRef<str>) -> MinilithResult<Self::OkType>;
}
pub trait MinilithErrorOptionExt {
    type SomeType;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_db`].
    ///
    /// # Errors
    ///
    /// Returns a [`MinilithEndpointError::InternalServerError`] with the appropriate metadata.
    fn wrap_err_encryption(self, subcode: impl AsRef<str>) -> MinilithResult<Self::SomeType>;
}
impl<T, E: Display> MinilithErrorResultExt for Result<T, E> {
    type OkType = T;
    fn wrap_err_bad_request(self, subcode: impl AsRef<str>) -> MinilithResult<Self::OkType> {
        self.map_err(|err| {
            MinilithEndpointError::bad_frontend_code(
                subcode,
                format!("Something went wrong! We received unexpected data: {err}"),
            )
        })
    }
    fn wrap_err_form(
        self,
        subcode: impl AsRef<str>,
        field: impl Into<String>,
    ) -> MinilithResult<Self::OkType> {
        self.map_err(|err| {
            MinilithEndpointError::bad_user_input(
                subcode,
                format!("Something went wrong! We received unexpected data: {err}"),
                field,
            )
        })
    }
    fn wrap_err_unauthorized(self, subcode: impl AsRef<str>) -> MinilithResult<Self::OkType> {
        self.map_err(|_err| {
            MinilithEndpointError::unauthorized(
                subcode,
                "unauthorized, try logging out and then in again",
            )
        })
    }
    fn wrap_err_not_found(self) -> MinilithResult<Self::OkType> {
        self.map_err(|_err| MinilithEndpointError::not_found())
    }
    fn wrap_err_db(self, subcode: impl AsRef<str>) -> MinilithResult<Self::OkType> {
        self.map_err(|err| {
            error!("DB error (DB_{}): {err}", subcode.as_ref());
            MinilithEndpointError::internal_error(
                subcode,
                MinilithErrorKind::Database,
                "database request failed",
            )
        })
    }
}
impl<T> MinilithErrorOptionExt for Option<T> {
    type SomeType = T;
    fn wrap_err_encryption(self, subcode: impl AsRef<str>) -> MinilithResult<Self::SomeType> {
        self.ok_or_else(|| {
            MinilithEndpointError::internal_error(
                subcode,
                MinilithErrorKind::EncryptionDecryption,
                "internal unrecoverable error",
            )
        })
    }
}

/// # Errors
///
/// See [`Context::new`].
pub async fn get_endpoint(test_db: Option<PgPool>) -> color_eyre::Result<impl Endpoint> {
    let context = Context::new(test_db).await?;
    let auth_context = AuthContext::new().await?;
    let api_service = OpenApiService::new(
        (
            activities::Router {
                context: context.clone(),
            },
            group::Router {
                context: context.clone(),
            },
            ticket::Router {
                context: context.clone(),
            },
            healthcheck::Router {
                context: context.clone(),
            },
            user::Router {
                context: context.clone(),
            },
        ),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server("http://localhost:8000/v0");
    let ui = api_service.swagger_ui();
    let spec = api_service.spec_endpoint();

    let cors = Cors::new()
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        .allow_header("content-type")
        .allow_header("authorization")
        .allow_credentials(true);

    Ok(Route::new()
        .nest("/v0", api_service.data(auth_context))
        .nest("/v0/docs", ui)
        .nest("/v0/spec.json", spec)
        .with(cors))
}
