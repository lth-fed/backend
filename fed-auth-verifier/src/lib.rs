use std::cell::{LazyCell, RefCell};

use jsonwebtoken::{Algorithm, Validation};
use poem::FromRequest;
use poem::http::StatusCode;
use poem_openapi::{ApiResponse, Object};
use serde::Deserialize;

const AUTH_KEY_URL: &str = "https://auth.teknologappen.se/api/v0/verify-key.der";

thread_local! {
    static AUTH_KEY: RefCell<Option<jsonwebtoken::DecodingKey>> = const { RefCell::new(None) };
    /// So we don't have to re-allocate for every check
    static VALIDATION: LazyCell<Validation> = LazyCell::new(|| {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_nbf = true;
        validation.set_audience(&["teknologappen.se"]);
        validation
    });
}
async fn assure_verification_key() -> Result<(), StatusCode> {
    if AUTH_KEY.with_borrow(Option::is_none) {
        let resp = reqwest::get(AUTH_KEY_URL)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if resp.status() != StatusCode::OK {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let der = resp
            .bytes()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        AUTH_KEY.with_borrow_mut(|opt| {
            let key = jsonwebtoken::DecodingKey::from_ed_der(&der);
            *opt = Some(key);
        });
    }
    Ok(())
}
const TESTING: Option<&str> = option_env!("TESTING");
fn is_testing() -> bool {
    matches!(TESTING, Some("true" | "yes" | "1"))
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
}

#[derive(Debug)]
pub struct User {
    id: String,
}
impl User {
    #[must_use]
    pub fn get_id(&self) -> &str {
        &self.id
    }
}

impl<'a> FromRequest<'a> for User {
    async fn from_request(
        req: &'a poem::Request,
        _body: &mut poem::RequestBody,
    ) -> poem::Result<Self> {
        if is_testing() {
            return Ok(Self {
                id: "lund-university:aa0000bb-s".to_owned(),
            });
        }

        let authorization = req
            .header("authorization")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let token = authorization
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;
        assure_verification_key().await?;
        let data: jsonwebtoken::TokenData<Claims> = AUTH_KEY
            .with_borrow(|key| {
                let Some(key) = key else {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                };
                Ok(VALIDATION.with(|validation| jsonwebtoken::decode(token, key, validation)))
            })?
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        Ok(Self {
            id: data.claims.sub,
        })
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
        _req: &'a poem::Request,
        body: &mut poem::RequestBody,
    ) -> poem::Result<Self> {
        assure_verification_key()
            .await
            .map_err(|_| CallbackResponseError::Unknown)?;
        let body = body.take()?;
        let body = body.into_string().await?;
        let data: CallbackDataV1 = AUTH_KEY
            .with_borrow(|key| {
                let Some(key) = key else {
                    return Err(CallbackResponseError::Unknown);
                };
                Ok(VALIDATION.with(|validation| jsonwebtoken::decode(&body, key, validation)))
            })?
            .map_err(|_| CallbackResponseError::SignatureInvalid)?
            .claims;
        Ok(data)
    }
}
