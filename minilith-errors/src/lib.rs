use std::fmt::{Debug, Display};
use std::str::FromStr as _;

use lettre::Transport as _;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object};
use tracing::error;

/// This assumes this crate (`minilith-errors`) is in the same repo as the rest of the backend!
pub const GIT_VERSION: &str = git_version::git_version!();

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
impl MinilithError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            field: None,
        }
    }
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
    pub fn inner(&self) -> &MinilithError {
        match self {
            MinilithEndpointError::BadRequest(json)
            | MinilithEndpointError::Unauthorized(json)
            | MinilithEndpointError::Forbidden(json)
            | MinilithEndpointError::NotFound(json)
            | MinilithEndpointError::InternalServerError(json) => &json.0,
        }
    }
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
    pub fn internal_error(error_message: impl AsRef<str>, error: impl Debug) -> Self {
        alert(
            AlertLevel::L3,
            format!(
                "internal error from wrap_err_internal, \
                message:<code>{}</code>, \
                error:<pre><code>{error:?}</code></pre>",
                error_message.as_ref()
            ),
        );
        // to get the trace
        error!(message = error_message.as_ref(), ?error, "Internal error.");
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
#[cfg(feature = "sqlx")]
impl From<sqlx::Error> for MinilithEndpointError {
    #[track_caller]
    fn from(error: sqlx::Error) -> Self {
        let level = if error.as_database_error().is_some() {
            // probably constraint
            AlertLevel::L3
        } else {
            // we can't connect to DB!
            AlertLevel::L2
        };
        alert(
            level,
            format!(
                "db error, \
                error:<pre><code>{error:?}</code></pre>",
            ),
        );
        Self::db(error)
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
/// Implemented for all [`Result`]s which has an error which implements [`Debug`].
pub trait MinilithErrorResultExt<T, E>: Sized {
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
    /// Calls [`MinilithEndpointError::internal_error`].
    /// Use `?` instead for [`sqlx::Error`].
    ///
    /// # Errors
    ///
    /// Maps the error to a [`MinilithEndpointError`].
    /// Does not leak the [`Display`] impl of the error to the client.
    fn wrap_err_internal(self, error_message: impl AsRef<str>) -> MinilithResult<T>;
}
/// Trait to make `.wrap_err_encryption()` work on [`Option`] types. Implemented for all
/// [`Option`]s.
pub trait MinilithErrorOptionExt<T>: Sized {
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
    fn wrap_err_internal(self, error_message: impl AsRef<str>) -> MinilithResult<T>;
    /// Same as [`MinilithErrorOptionExt::wrap_err_internal`] but with an encryption error message
    /// attached.
    ///
    /// # Errors
    ///
    /// Returns a [`MinilithEndpointError::InternalServerError`] with the appropriate metadata.
    fn wrap_err_encryption(self, error_message: impl AsRef<str>) -> MinilithResult<T> {
        self.wrap_err_internal(format!(
            "encryption / decryption error: {}",
            error_message.as_ref()
        ))
    }
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
    fn wrap_err_internal(self, error_message: impl AsRef<str>) -> MinilithResult<T> {
        // inlined so track_caller works as expected
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(MinilithEndpointError::internal_error(error_message, error)),
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
    fn wrap_err_internal(self, error_message: impl AsRef<str>) -> MinilithResult<T> {
        self.ok_or_else(|| MinilithEndpointError::internal_error(error_message, ""))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AlertLevel {
    /// For the most critical things. This could be errors with payments such that someone could
    /// lose money.
    L1,
    /// Critical systems are not working or can not talk. Not for individual application errors,
    /// rather for very critical such and connection errors.
    L2,
    /// For all random internal server errors. Our goal is for these to never exist.
    L3,
    /// Use this for testing this system at startup.
    DryRun,
}
impl Display for AlertLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertLevel::L1 => f.write_str("LEVEL 1"),
            AlertLevel::L2 => f.write_str("LEVEL 2"),
            AlertLevel::L3 => f.write_str("LEVEL 3"),
            AlertLevel::DryRun => f.write_str("DRY RUN (for testing the alert system)"),
        }
    }
}

trait AlertResultExt<T> {
    fn wrap(self, message: impl AsRef<str>) -> Result<T, ()>;
}
impl<T, E: Debug> AlertResultExt<T> for Result<T, E> {
    fn wrap(self, message: impl AsRef<str>) -> Result<T, ()> {
        self.inspect_err(|error| error!(?error, "ALERTS: {}", message.as_ref()))
            .map_err(|_| ())
    }
}
/// This assumes this crate (`minilith-errors`) is in the same repo as the rest of the backend!
pub fn alert(level: AlertLevel, message: impl AsRef<str>) {
    let _: Result<(), ()> = alert_inner(level, message);
}
fn alert_inner(level: AlertLevel, message: impl AsRef<str>) -> Result<(), ()> {
    let recipients = match level {
        AlertLevel::L1 => dotenvy::var("ALERT_RECIPIENTS_LEVEL_1"),
        AlertLevel::L2 => dotenvy::var("ALERT_RECIPIENTS_LEVEL_2"),
        AlertLevel::L3 => dotenvy::var("ALERT_RECIPIENTS_LEVEL_3"),
        AlertLevel::DryRun => dotenvy::var("ALERT_RECIPIENTS_LEVEL_1")
            .and_then(|_| dotenvy::var("ALERT_RECIPIENTS_LEVEL_2"))
            .and_then(|_| dotenvy::var("ALERT_RECIPIENTS_LEVEL_3"))
            .map(|_| String::new()),
    };
    let recipients = recipients
        .wrap("Failed to load ALERT_RECIPIENTS_LEVEL_<level>. Assure they are available.")?;
    let recipients = recipients.lines();

    let from = dotenvy::var("ALERT_EMAIL").wrap("No ALERT_EMAIL variable")?;
    let password = dotenvy::var("ALERT_PASSWORD").wrap("No ALERT_PASSWORD variable")?;
    let smtp = dotenvy::var("ALERT_SMTP").wrap("No ALERT_SMTP variable")?;

    let email =
        lettre::Address::from_str(&from).wrap("ALERT_EMAIL is not a valid email address")?;
    let mailbox = lettre::message::Mailbox::new(Some("Alerts from Teknologappen".into()), email);

    let mut msg = lettre::Message::builder()
        .from(mailbox)
        .subject(format!("ALERT {level} from teknologappen"))
        .header(lettre::message::header::ContentType::TEXT_HTML);

    for recipient in recipients {
        let mbox = lettre::message::Mailbox::from_str(recipient)
            .wrap("ALERT_RECIPIENTS_LEVEL_<x> contains an invalid email: {recipient}")?;
        msg = msg.to(mbox);
    }
    let credentials =
        lettre::transport::smtp::authentication::Credentials::new(from.clone(), password);
    let transport = lettre::SmtpTransport::relay(&smtp)
        .wrap("Failed to connect via SMTPS to ALERT_SMTP")?
        .credentials(credentials)
        .authentication(vec![
            lettre::transport::smtp::authentication::Mechanism::Plain,
        ])
        .build();

    let trace = std::backtrace::Backtrace::force_capture();
    let message = message.as_ref();
    let intro = match level {
        AlertLevel::L1 => {
            "A level 1 critical error occurred in any of the teknologappen instances. \
            Please fix this as soon as possible and contact the impacted."
        }
        AlertLevel::L2 => {
            "A level 2 critical error occurred in any of the teknologappen instances. \
            Please fix this within a few hours and contact the impacted."
        }
        AlertLevel::L3 => {
            "A level 3 critical error occurred in any of the teknologappen instances. \
            Please fix this within the week. If more errors occurr, contact the impacted."
        }
        // here we stop the dry run, before we send the mail!
        AlertLevel::DryRun => return Ok(()),
    };
    let msg = msg
        .body(format!(
            "<p>
        {intro}
        </p>
        <p>
            A message was included from the code: <code>{message}</code>.
            <b>Version</b>: <code>{GIT_VERSION}</code>.
        </p>
        <p>
            To ease debugging the backtrace is inserted below.
            <br>
            <pre><code>{trace}</code></pre>
        </p>"
        ))
        .wrap("Failed to attach a body to the message")?;

    let resp = transport.send(&msg).wrap("Failed to send message!")?;
    if !resp.is_positive() {
        error!(status=%resp.code(), "ALERTS: failed to send mail!");
    }
    Ok(())
}
pub fn test_alerts() {
    alert(AlertLevel::DryRun, "test");
}
