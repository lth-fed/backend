use std::fmt::{Debug, Display};
use std::str::FromStr as _;
use std::sync::OnceLock;

use color_eyre::eyre::{Context as _, bail};
use lettre::AsyncTransport as _;
use poem_openapi::payload::Json;
use poem_openapi::{ApiResponse, Object};
use tracing::error;

/// This assumes this crate (`minilith-errors`) is in the same repo as the rest of the backend!
pub const GIT_VERSION: &str = if let Some(version) = option_env!("GIT_VERSION") {
    version
} else {
    git_version::git_version!(fallback = "unknown")
};

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

/// A reusable SMTP client configured from environment variables.
///
/// For a prefix such as `MAIL`, the expected variables are `MAIL_SMTP`,
/// `MAIL_EMAIL`, and `MAIL_PASSWORD`. If all three are absent, email is
/// disabled and [`EmailClient::new`] returns `Ok(None)`. A partial
/// configuration is an error.
#[derive(Clone)]
pub struct EmailClient {
    from: lettre::Address,
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
}

impl Debug for EmailClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailClient")
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

impl EmailClient {
    /// Creates an SMTP client from `<prefix>_SMTP`, `<prefix>_EMAIL`, and
    /// `<prefix>_PASSWORD`.
    ///
    /// # Errors
    ///
    /// Returns an error for a partial configuration, an invalid sender
    /// address, or an invalid SMTP relay.
    pub fn new(prefix: &str) -> color_eyre::Result<Option<Self>> {
        let smtp_key = format!("{prefix}_SMTP");
        let email_key = format!("{prefix}_EMAIL");
        let password_key = format!("{prefix}_PASSWORD");

        let smtp = dotenvy::var(&smtp_key);
        let email = dotenvy::var(&email_key);
        let password = dotenvy::var(&password_key);
        if matches!(
            smtp,
            Err(dotenvy::Error::EnvVar(std::env::VarError::NotPresent))
        ) && matches!(
            email,
            Err(dotenvy::Error::EnvVar(std::env::VarError::NotPresent))
        ) && matches!(
            password,
            Err(dotenvy::Error::EnvVar(std::env::VarError::NotPresent))
        ) {
            return Ok(None);
        }

        let smtp = smtp.wrap_err_with(|| format!("`{smtp_key}` not set"))?;
        let email = email.wrap_err_with(|| format!("`{email_key}` not set"))?;
        let password = password.wrap_err_with(|| format!("`{password_key}` not set"))?;
        let from = email
            .parse()
            .wrap_err_with(|| format!("`{email_key}` is not a valid email address"))?;
        let credentials =
            lettre::transport::smtp::authentication::Credentials::new(email, password);
        let transport = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::relay(&smtp)
            .wrap_err_with(|| format!("failed to configure the `{prefix}` SMTP relay"))?
            .credentials(credentials)
            .authentication(vec![
                lettre::transport::smtp::authentication::Mechanism::Plain,
            ])
            .build();

        Ok(Some(Self { from, transport }))
    }

    /// Sends one HTML-only message to every supplied recipient.
    ///
    /// # Errors
    ///
    /// Returns an error if there are no recipients, an address or message is
    /// invalid, or the SMTP server rejects delivery.
    pub async fn send_html<'a>(
        &self,
        from_name: &str,
        to: impl IntoIterator<Item = &'a str>,
        subject: &str,
        html: impl Into<String>,
    ) -> color_eyre::Result<()> {
        let mut message = lettre::Message::builder()
            .from(lettre::message::Mailbox::new(
                Some(from_name.to_owned()),
                self.from.clone(),
            ))
            .subject(subject)
            .header(lettre::message::header::ContentType::TEXT_HTML);

        let mut has_recipient = false;
        for recipient in to {
            let mailbox = lettre::message::Mailbox::from_str(recipient)
                .wrap_err_with(|| format!("invalid recipient email address: {recipient}"))?;
            message = message.to(mailbox);
            has_recipient = true;
        }
        if !has_recipient {
            bail!("cannot send an email without recipients");
        }

        let message = message
            .body(html.into())
            .wrap_err("failed to format email")?;
        let response = self
            .transport
            .send(message)
            .await
            .wrap_err("failed to send email")?;
        if !response.is_positive() {
            bail!("SMTP server rejected email with status {}", response.code());
        }
        Ok(())
    }
}

/// Escapes untrusted text before inserting it into an HTML email.
#[must_use]
pub fn escape_email_html(text: &str) -> std::borrow::Cow<'_, str> {
    html_escape::encode_text(text)
}

static ALERT_EMAIL_CLIENT: OnceLock<EmailClient> = OnceLock::new();

fn alert_recipients(level: AlertLevel) -> color_eyre::Result<Vec<String>> {
    let variable = match level {
        AlertLevel::L1 | AlertLevel::DryRun => "ALERT_RECIPIENTS_LEVEL_1",
        AlertLevel::L2 => "ALERT_RECIPIENTS_LEVEL_2",
        AlertLevel::L3 => "ALERT_RECIPIENTS_LEVEL_3",
    };
    let recipients = dotenvy::var(variable).wrap_err_with(|| format!("`{variable}` not set"))?;
    let recipients: Vec<_> = recipients
        .lines()
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
        .map(str::to_owned)
        .collect();
    if recipients.is_empty() {
        bail!("`{variable}` contains no recipients");
    }
    for recipient in &recipients {
        lettre::message::Mailbox::from_str(recipient)
            .wrap_err_with(|| format!("`{variable}` contains an invalid email: {recipient}"))?;
    }
    Ok(recipients)
}

/// Makes an already-created client available to the process-wide alert helper.
///
/// All alert recipient variables are validated here so configuration failures
/// are reported by `Context::new`, rather than on the first production error.
///
/// # Errors
///
/// Returns an error if any alert recipient list is missing or invalid.
pub fn configure_alert_email(client: Option<EmailClient>) -> color_eyre::Result<()> {
    let Some(client) = client else {
        return Ok(());
    };
    for level in [AlertLevel::L1, AlertLevel::L2, AlertLevel::L3] {
        drop(alert_recipients(level)?);
    }
    drop(ALERT_EMAIL_CLIENT.set(client));
    Ok(())
}

/// Sends an operational alert in English.
///
/// Delivery is scheduled on the application's existing Tokio runtime so this
/// function remains usable by synchronous error-conversion code.
pub fn alert(level: AlertLevel, message: impl AsRef<str>) {
    if matches!(level, AlertLevel::DryRun) {
        return;
    }
    let Some(client) = ALERT_EMAIL_CLIENT.get().cloned() else {
        error!(%level, "ALERTS: email is not configured");
        return;
    };
    let recipients = match alert_recipients(level) {
        Ok(recipients) => recipients,
        Err(error) => {
            error!(?error, %level, "ALERTS: failed to load recipients");
            return;
        }
    };

    let trace = std::backtrace::Backtrace::force_capture();
    let message = escape_email_html(message.as_ref());
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
        AlertLevel::DryRun => return,
    };
    let subject = format!("ALERT {level} from teknologappen");
    let html = format!(
        "<p>{intro}</p>\
         <p>A message was included from the code: <code>{message}</code>. \
         <strong>Version</strong>: <code>{GIT_VERSION}</code>.</p>\
         <p>To ease debugging the backtrace is inserted below.</p>\
         <pre><code>{}</code></pre>",
        escape_email_html(&trace.to_string()),
    );
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        error!(%level, "ALERTS: no Tokio runtime is available for email delivery");
        return;
    };
    drop(runtime.spawn(async move {
        if let Err(error) = client
            .send_html(
                "Alerts from Teknologappen",
                recipients.iter().map(String::as_str),
                &subject,
                html,
            )
            .await
        {
            error!(?error, %level, "ALERTS: failed to send email");
        }
    }));
}

/// Validates alert recipient configuration without sending any email.
pub fn test_alerts() {
    for level in [AlertLevel::L1, AlertLevel::L2, AlertLevel::L3] {
        if let Err(error) = alert_recipients(level) {
            error!(?error, "ALERTS: invalid recipient configuration");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::escape_email_html;

    #[test]
    fn escapes_email_html_text() {
        assert_eq!(
            escape_email_html("<admin's \"group\" & users>"),
            "&lt;admin's \"group\" &amp; users&gt;",
        );
    }
}
