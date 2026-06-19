use std::fmt::Display;

use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Enum, Object};
use tracing::error;
use uuid::Uuid;

pub type MinilithResult<T> = Result<T, MinilithEndpointError>;

/// Which kind of error this is. Similar to HTTP status codes but more comprehensive and specific.
#[derive(Enum, Debug, Clone, Copy)]
pub enum MinilithErrorKind {
    /// Code prefix: `BF`.
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
    /// Append a subcode to make a full code with [`Self::as_code_prefix`].
    pub fn to_full_code(&self, subcode: impl AsRef<str>) -> String {
        format!("{}_{}", self.as_code_prefix(), subcode.as_ref())
    }
    /// This error kind's prefix. See the options of [`MinilithErrorKind`].
    #[must_use]
    pub fn as_code_prefix(&self) -> &'static str {
        match self {
            MinilithErrorKind::BadFrontend => "BF",
            MinilithErrorKind::BadUser => "BU",
            MinilithErrorKind::Unauthorized => "AUTH",
            MinilithErrorKind::NotFound => "NF",
            MinilithErrorKind::Database => "DB",
            MinilithErrorKind::EncryptionDecryption => "ENC",
            MinilithErrorKind::Other => "UK",
        }
    }
}
/// An error object with an error code, an error kind (which is also encoded in to the code), a
/// message, and optionally a field to signify where in e.g. a form the error came from in
/// validation.
///
/// This struct is most often created by calling static methods on [`MinilithEndpointError`].
#[derive(Object, Debug)]
#[non_exhaustive]
#[must_use]
pub struct MinilithError {
    pub id: Uuid,
    pub code: String,
    pub kind: MinilithErrorKind,
    pub message: String,
    pub field: Option<String>,
}

/// The error kind returned from most handlers. Really just [`MinilithError`] but wrapped in poem
/// stuff to make it look nice in `OpenAPI` & HTTP.
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
    #[must_use]
    pub fn id(&self) -> Uuid {
        match self {
            MinilithEndpointError::BadRequest(err)
            | MinilithEndpointError::Unauthorized(err)
            | MinilithEndpointError::NotFound(err)
            | MinilithEndpointError::InternalServerError(err) => err.0.id,
        }
    }
    /// For when the frontend didn't uphold a contract.
    pub fn bad_frontend_code(subcode: impl AsRef<str>, message: impl Into<String>) -> Self {
        Self::BadRequest(Json(MinilithError {
            id: Uuid::new_v4(),
            code: MinilithErrorKind::BadFrontend.to_full_code(subcode),
            kind: MinilithErrorKind::BadFrontend,
            message: message.into(),
            field: None,
        }))
    }
    /// For when the user did something stupid (e.g. validation failure).
    ///
    /// The absolute most cases involve [`Self::bad_frontend_code`] instead, since the frontend is
    /// expected to foresee a bunch of the errors.
    pub fn bad_user_input(
        subcode: impl AsRef<str>,
        message: impl Into<String>,
        field: impl Into<String>,
    ) -> Self {
        Self::BadRequest(Json(MinilithError {
            id: Uuid::new_v4(),
            code: MinilithErrorKind::BadUser.to_full_code(subcode),
            kind: MinilithErrorKind::BadUser,
            message: message.into(),
            field: Some(field.into()),
        }))
    }
    /// Only to be used for actual auth problems, not for when access is limited.
    ///
    /// When access is limited, [`Self::bad_frontend_code`] is often better.
    pub fn unauthorized(subcode: impl AsRef<str>, message: impl Into<String>) -> Self {
        Self::Unauthorized(Json(MinilithError {
            id: Uuid::new_v4(),
            code: MinilithErrorKind::Unauthorized.to_full_code(subcode),
            kind: MinilithErrorKind::Unauthorized,
            message: message.into(),
            field: None,
        }))
    }
    /// This resource wasn't found. Is arguably [`Self::bad_frontend_code`]. Should they be merged?
    pub fn not_found() -> Self {
        Self::NotFound(Json(MinilithError {
            id: Uuid::new_v4(),
            code: MinilithErrorKind::NotFound.as_code_prefix().into(),
            kind: MinilithErrorKind::NotFound,
            message: "resource not found, try reloading app".into(),
            field: None,
        }))
    }
    /// An internal error happened! Please only use a reasonable [`MinilithErrorKind`], like
    /// [`MinilithErrorKind::Database`] or [`MinilithErrorKind::EncryptionDecryption`].
    pub fn internal_error(
        subcode: impl AsRef<str>,
        kind: MinilithErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self::InternalServerError(Json(MinilithError {
            id: Uuid::new_v4(),
            code: if subcode.as_ref() == "" {
                kind.as_code_prefix().to_owned()
            } else {
                kind.to_full_code(subcode)
            },
            kind,
            message: message.into(),
            field: None,
        }))
    }
    pub fn db<E: Display>(err: E) -> Self {
        let me = Self::internal_error("", MinilithErrorKind::Database, "database request failed");
        error!("DB error ({}): {err}", me.id());
        me
    }
}
/// Poem thing.
#[allow(
    clippy::needless_pass_by_value,
    reason = "poem requires us to consume it"
)]
fn bad_request_handler(err: poem::Error) -> MinilithEndpointError {
    // the errors we get from poem are when it's parsing the parameters
    MinilithEndpointError::bad_frontend_code(
        "PARAM",
        format!("Something went wrong! We received unexpected data: {err}"),
    )
}
/// Trait to make `.wrap_err_db()` work on [`Result`] types. Implemented for all [`Result`]s which
/// has an error which implements [`Display`].
pub trait MinilithErrorResultExt<T, E> {
    /// Subcode should be globally unique. A common way to do this is to describe what it does but
    /// very shortened. Say we're wrapping this in the activities list handler, subcode could be
    /// `ACT_USER_LIST`.
    ///
    /// Subcode is called subcode and not code because a prefix is added to make it a complete code
    /// (`DB_` in this case).
    ///
    /// Calls [`MinilithEndpointError::bad_frontend_code`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Leaks the [`Display`] impl of the error to the client.
    fn wrap_err_bad_frontend(self, subcode: impl AsRef<str>) -> MinilithResult<T>;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_bad_frontend`].
    /// Calls [`MinilithEndpointError::bad_user_input`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_bad_user(
        self,
        subcode: impl AsRef<str>,
        field: impl Into<String>,
    ) -> MinilithResult<T>;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_bad_frontend`].
    /// Calls [`MinilithEndpointError::unauthorized`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_unauthorized(self, subcode: impl AsRef<str>) -> MinilithResult<T>;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_bad_frontend`].
    /// Calls [`MinilithEndpointError::not_found`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_not_found(self) -> MinilithResult<T>;
    /// Calls [`MinilithEndpointError::internal_error`] with [`MinilithErrorKind::Database`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_db(self) -> MinilithResult<T>;
}
/// Trait to make `.wrap_err_encryption()` work on [`Option`] types. Implemented for all
/// [`Option`]s.
pub trait MinilithErrorOptionExt<T> {
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_bad_frontend`].
    ///
    /// # Errors
    ///
    /// Returns a [`MinilithEndpointError::BadRequest`] with the appropriate metadata.
    fn wrap_err_bad_frontend(
        self,
        subcode: impl AsRef<str>,
        err: impl AsRef<str>,
    ) -> MinilithResult<T>;
    /// `subcode`: see [`MinilithErrorResultExt::wrap_err_bad_frontend`].
    ///
    /// # Errors
    ///
    /// Returns a [`MinilithEndpointError::InternalServerError`] with the appropriate metadata.
    fn wrap_err_encryption(self, subcode: impl AsRef<str>) -> MinilithResult<T>;
}
impl<T, E: Display> MinilithErrorResultExt<T, E> for Result<T, E> {
    fn wrap_err_bad_frontend(self, subcode: impl AsRef<str>) -> MinilithResult<T> {
        self.map_err(|err| {
            MinilithEndpointError::bad_frontend_code(
                subcode,
                format!("Something went wrong! We received unexpected data: {err}"),
            )
        })
    }
    fn wrap_err_bad_user(
        self,
        subcode: impl AsRef<str>,
        field: impl Into<String>,
    ) -> MinilithResult<T> {
        self.map_err(|err| {
            MinilithEndpointError::bad_user_input(
                subcode,
                format!("Something went wrong! We received unexpected data: {err}"),
                field,
            )
        })
    }
    fn wrap_err_unauthorized(self, subcode: impl AsRef<str>) -> MinilithResult<T> {
        self.map_err(|_err| {
            MinilithEndpointError::unauthorized(
                subcode,
                "unauthorized, try logging out and then in again",
            )
        })
    }
    fn wrap_err_not_found(self) -> MinilithResult<T> {
        self.map_err(|_err| MinilithEndpointError::not_found())
    }
    #[track_caller]
    fn wrap_err_db(self) -> MinilithResult<T> {
        // inlined so track_caller works as expected
        match self {
            Ok(value) => Ok(value),
            Err(err) => Err(MinilithEndpointError::db(err)),
        }
    }
}
impl<T> MinilithErrorOptionExt<T> for Option<T> {
    fn wrap_err_bad_frontend(
        self,
        subcode: impl AsRef<str>,
        err: impl AsRef<str>,
    ) -> MinilithResult<T> {
        self.ok_or_else(|| {
            MinilithEndpointError::bad_frontend_code(
                subcode,
                format!(
                    "Something went wrong! We received unexpected data: {}",
                    err.as_ref()
                ),
            )
        })
    }
    #[track_caller]
    fn wrap_err_encryption(self, subcode: impl AsRef<str>) -> MinilithResult<T> {
        // inlined so track_caller works as expected
        if let Some(value) = self {
            Ok(value)
        } else {
            let err = MinilithEndpointError::internal_error(
                subcode,
                MinilithErrorKind::EncryptionDecryption,
                "internal unrecoverable error",
            );
            error!("Encryption / decryption error ({})!!!", err.id());
            Err(err)
        }
    }
}
