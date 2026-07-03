use std::fmt::Debug;

use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object};
use tracing::error;

pub type MinilithResult<T> = Result<T, MinilithEndpointError>;

/// An error object a message, and optionally a field to signify where in e.g. a form the error
/// came from in validation.
///
/// This struct is most often created by calling static methods on [`MinilithEndpointError`].
#[derive(Object, Debug)]
#[non_exhaustive]
#[must_use]
pub struct MinilithError {
    pub message: String,
    pub field: Option<String>,
}

/// The error kind returned from most handlers. Really just [`MinilithError`] but wrapped in poem
/// stuff to make it look nice in `OpenAPI` & HTTP.
#[derive(ApiResponse, Debug)]
#[oai(bad_request_handler = "bad_request_handler")]
#[must_use]
pub enum MinilithEndpointError {
    /// This is for user input errors.
    #[oai(status = 400)]
    BadRequest(Json<MinilithError>),
    /// This is for auth errors. This usually requires re-login.
    #[oai(status = 401)]
    Unauthorized(Json<MinilithError>),
    /// This is for client application errors.
    #[oai(status = 403)]
    Forbidden(Json<MinilithError>),
    /// This is for when the user requests something that doesn't exist. Probably cache invalidaton
    /// issue.
    #[oai(status = 404)]
    NotFound(Json<MinilithError>),
    /// Shit went down and the team is scrambling to fix it.
    #[oai(status = 500)]
    InternalServerError(Json<MinilithError>),
}
impl MinilithEndpointError {
    /// For when the frontend didn't uphold a contract.
    #[track_caller]
    pub fn bad_frontend_code(error_message: impl AsRef<str>, error: impl Debug) -> Self {
        // to get the trace
        error!(
            ?error,
            message = error_message.as_ref(),
            "Bad frontend code."
        );
        Self::Forbidden(Json(MinilithError {
            message: "contact app developers".into(),
            field: None,
        }))
    }
    /// For when the user did something stupid (e.g. validation failure).
    ///
    /// The absolute most cases involve [`Self::bad_frontend_code`] instead, since the frontend is
    /// expected to foresee a bunch of the errors.
    #[track_caller]
    pub fn bad_user_input(
        error_message: impl AsRef<str>,
        error: impl Debug,
        user_message: impl Into<String>,
        field: impl Into<String>,
    ) -> Self {
        // to get the trace
        let user_message = user_message.into();
        error!(
            user_message = user_message,
            error_message = error_message.as_ref(),
            ?error,
            "Bad user input"
        );
        Self::BadRequest(Json(MinilithError {
            message: user_message,
            field: Some(field.into()),
        }))
    }
    /// Only to be used for actual auth problems, not for when access is limited.
    ///
    /// When access is limited, [`Self::bad_frontend_code`] is often better.
    #[track_caller]
    pub fn unauthorized(error_message: impl AsRef<str>, error: impl Debug) -> Self {
        // to get the trace
        error!(?error, message = error_message.as_ref(), "Unauthorized.");
        Self::Unauthorized(Json(MinilithError {
            message: "try logging out and then in again or contact app developers".into(),
            field: None,
        }))
    }
    /// This resource wasn't found. Is arguably [`Self::bad_frontend_code`]. Should they be merged?
    #[track_caller]
    pub fn not_found() -> Self {
        // to get the trace
        error!("Not found.");
        Self::NotFound(Json(MinilithError {
            message: "resource not found, try reloading app".into(),
            field: None,
        }))
    }
    /// An internal error happened! You MUST nog an error too.
    #[track_caller]
    pub fn internal_error(error_message: impl AsRef<str>) -> Self {
        // to get the trace
        error!(message = error_message.as_ref(), "Internal error.");
        Self::InternalServerError(Json(MinilithError {
            message: "Something went very wrong. Contact app developers.".to_owned(),
            field: None,
        }))
    }
    #[track_caller]
    pub fn db<E: Debug>(error: E) -> Self {
        error!(?error, "DB error");
        Self::InternalServerError(Json(MinilithError {
            message: "Something went very wrong. Contact app developers. Database request failed."
                .to_owned(),
            field: None,
        }))
    }
}
/// Poem thing.
#[allow(
    clippy::needless_pass_by_value,
    reason = "poem requires us to consume it"
)]
#[track_caller]
fn bad_request_handler(error: poem::Error) -> MinilithEndpointError {
    // the errors we get from poem are when it's parsing the parameters
    MinilithEndpointError::bad_frontend_code("Param error", error)
}
/// Trait to make `.wrap_err_db()` work on [`Result`] types. Implemented for all [`Result`]s which
/// has an error which implements [`Display`].
pub trait MinilithErrorResultExt<T, E> {
    /// Calls [`MinilithEndpointError::bad_frontend_code`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Leaks the [`Display`] impl of the error to the client.
    fn wrap_err_bad_frontend(self, error_message: impl AsRef<str>) -> MinilithResult<T>;
    /// Calls [`MinilithEndpointError::bad_user_input`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_bad_user(
        self,
        error_message: impl AsRef<str>,
        field: impl Into<String>,
    ) -> MinilithResult<T>;
    /// Calls [`MinilithEndpointError::unauthorized`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_unauthorized(self, error_message: impl AsRef<str>) -> MinilithResult<T>;
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
    /// # Errors
    ///
    /// Returns a [`MinilithEndpointError::BadRequest`] with the appropriate metadata.
    fn wrap_err_bad_frontend(self, error_message: impl AsRef<str>) -> MinilithResult<T>;
    /// Calls [`MinilithEndpointError::not_found`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_not_found(self) -> MinilithResult<T>;
    /// # Errors
    ///
    /// Returns a [`MinilithEndpointError::InternalServerError`] with the appropriate metadata.
    fn wrap_err_encryption(self, error_message: impl AsRef<str>) -> MinilithResult<T>;
}
impl<T, E: Debug> MinilithErrorResultExt<T, E> for Result<T, E> {
    #[track_caller]
    fn wrap_err_bad_frontend(self, error_message: impl AsRef<str>) -> MinilithResult<T> {
        self.map_err(|error| MinilithEndpointError::bad_frontend_code(error_message, error))
    }
    #[track_caller]
    fn wrap_err_bad_user(
        self,
        error_message: impl AsRef<str>,
        field: impl Into<String>,
    ) -> MinilithResult<T> {
        self.map_err(|error| {
            MinilithEndpointError::bad_user_input(
                error_message,
                error,
                "Something went wrong! We received unexpected data.",
                field,
            )
        })
    }
    #[track_caller]
    fn wrap_err_unauthorized(self, error_message: impl AsRef<str>) -> MinilithResult<T> {
        self.map_err(|error| MinilithEndpointError::unauthorized(error_message, error))
    }
    #[track_caller]
    fn wrap_err_db(self) -> MinilithResult<T> {
        // inlined so track_caller works as expected
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(MinilithEndpointError::db(error)),
        }
    }
}
impl<T> MinilithErrorOptionExt<T> for Option<T> {
    #[track_caller]
    fn wrap_err_bad_frontend(self, error_message: impl AsRef<str>) -> MinilithResult<T> {
        self.ok_or_else(|| MinilithEndpointError::bad_frontend_code(error_message, ""))
    }
    #[track_caller]
    fn wrap_err_not_found(self) -> MinilithResult<T> {
        self.ok_or_else(|| MinilithEndpointError::not_found())
    }
    #[track_caller]
    fn wrap_err_encryption(self, error_message: impl AsRef<str>) -> MinilithResult<T> {
        self.ok_or_else(|| {
            MinilithEndpointError::internal_error(format!(
                "encryption / decryption error: {}",
                error_message.as_ref()
            ))
        })
    }
}
