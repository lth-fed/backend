use jsonwebtoken::{Algorithm, Validation};
use poem::error::ResponseError;
use poem::http::StatusCode;
use poem::{FromRequest, IntoResponse as _};
use poem_openapi::auth::Bearer;
use poem_openapi::{ApiResponse, Object, SecurityScheme};
use serde::Deserialize;
use tracing::error;

const AUTH_KEY_URL: &str = "https://auth.teknologappen.se/api/v0/verifying-key";
const TESTING: Option<&str> = option_env!("TESTING");
fn is_testing() -> bool {
    matches!(TESTING, Some("true" | "yes" | "1"))
}

#[derive(Clone, Debug)]
#[must_use]
pub struct AuthContext {
    auth_key: jsonwebtoken::DecodingKey,
    validation: Validation,
}
impl AuthContext {
    /// This can not be used by `fed-auth`, since that's the service which this depends on!
    ///
    /// # Errors
    ///
    /// Returns an error if it was not possible to get the verifying key.
    pub async fn new() -> color_eyre::Result<Self> {
        let resp = reqwest::get(AUTH_KEY_URL).await?;
        if resp.status() != StatusCode::OK {
            return Err(color_eyre::eyre::Error::msg("failed getting verifying key"));
        }
        let der = resp.bytes().await?;
        if der.len() != 32 {
            return Err(color_eyre::eyre::Error::msg(
                "verifying key is wrong length",
            ));
        }
        let auth_key = jsonwebtoken::DecodingKey::from_ed_der(&der);
        Ok(Self::from_decoding_key(auth_key))
    }
    pub fn from_decoding_key(auth_key: jsonwebtoken::DecodingKey) -> Self {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_nbf = true;
        validation.set_audience(&["teknologappen.se"]);

        Self {
            auth_key,
            validation,
        }
    }
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Error {
    Unauthorized,
    InternalError,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.status(), f)
    }
}
impl std::error::Error for Error {}
impl ResponseError for Error {
    fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    fn as_response(&self) -> poem::Response
    where
        Self: std::error::Error + Send + Sync + 'static,
    {
        self.status().into_response()
    }
}

/// Returns the logged in user. Spec says OAuth2 but it really is OIDC but without automatic
/// discovery.
///
/// [`AuthContext`] MUST BE registered using [`poem::EndpointExt::data`].
///
/// # Errors
///
/// [`Error`]. This is mapped in `minilith` to it's errors.
#[derive(Debug, SecurityScheme)]
#[oai(
    ty = "oauth2",
    key_in = "header",
    key_name = "authorization",
    bearer_format = "JWT",
    flows(authorization_code(
        authorization_url = "https://auth.teknologappen.se/oidc/authorize",
        token_url = "https://auth.teknologappen.se/oidc/token",
    )),
    checker = "User::from_token"
)]
pub struct User(String);
impl User {
    #[must_use]
    pub fn get_id(&self) -> &str {
        &self.0
    }
    #[allow(clippy::unused_async, reason = "poem_openapi wants us to")]
    async fn from_token(req: &poem::Request, token: Bearer) -> poem::Result<String> {
        if is_testing() {
            return Ok("lund-university:aa0000bb-s".to_owned());
        }
        let context: &AuthContext = req.data().ok_or_else(|| {
            error!("AuthContext not registered as data!");
            Error::InternalError
        })?;

        let data =
            jsonwebtoken::decode::<Claims>(&token.token, &context.auth_key, &context.validation)
                .map_err(|_| Error::Unauthorized)?;
        Ok(data.claims.sub)
    }
}

#[derive(ApiResponse, Clone, Copy, Debug)]
pub enum CallbackResponseError {
    /// The request's signature was invalid; we can't trust this request.
    #[oai(status = 401)]
    SignatureInvalid,
    /// The DB threw an error.
    #[oai(status = 500)]
    DbError,
    /// Unknown internal error.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Debug, Object, Deserialize)]
pub struct CallbackDataV1 {
    pub sub: String,
    pub email: String,
    pub full_name: String,
}

/// [`AuthContext`] MUST BE registered using [`poem::EndpointExt::data`].
///
/// # Example
///
/// ```no_compile
/// # use poem_openapi::OpenApi;
/// # use fed_auth_verifier::CallbackDataV1;
/// pub struct Context {
///     db: sqlx::PgPool,
/// }
/// pub struct Router {
///     context: Context,
/// }
/// #[OpenApi]
/// impl Router {
///     #[oai(path = "/callback/v1", method = "post")]
///     async fn callback(&self, data: CallbackDataV1) {
///         sqlx::query!(
///             "insert into users values ($1, $2, $3)",
///             data.sub, data.full_name, data.mail,
///         )
///         .execute(&self.context.db)
///         .await
///         .unwrap();
///     }
/// }
/// ```
impl<'a> FromRequest<'a> for CallbackDataV1 {
    async fn from_request(
        req: &'a poem::Request,
        body: &mut poem::RequestBody,
    ) -> poem::Result<Self> {
        let context: &AuthContext = req.data().ok_or_else(|| {
            error!("AuthContext not registered as data!");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        let body = body.take()?;
        let body = body.into_string().await?;
        let data: CallbackDataV1 =
            jsonwebtoken::decode(&body, &context.auth_key, &context.validation)
                .map_err(|_| CallbackResponseError::SignatureInvalid)?
                .claims;
        Ok(data)
    }
}
