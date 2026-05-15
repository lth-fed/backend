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
pub struct CallbackData {
    pub sub: String,
    pub email: String,
    pub full_name: String,
}

/// $cb can use the try operator `?` for returning [`CallbackResponseError`].
///
/// # Example
///
/// ```no_compile
/// pub struct Context {
///     db: sqlx::PgPool,
/// }
/// auth_callback_router!(AuthRouter, Context, "/api/v0/auth-login", async |ctx, data| {
///     sqlx::query!(
///         "insert into users values ($1, $2, $3)",
///         data.sub, data.full_name, data.mail,
///     )
///     .execute(&*ctx.db)
///     .await
///     .map_err(|_| CallbackResponseError::DbError)?;
/// });
/// ```
#[macro_export]
macro_rules! auth_callback_router {
    ($router_name: ident, $context_type: ident, $url: literal, async |$context: ident, $data: ident| $cb: expr) => {
        use poem_openapi::{OpenApi, payload::PlainText};
        use std::ops::Deref;
        use $crate::*;

        pub struct $router_name {
            pub context: $context_type,
        }
        impl Deref for $router_name {
            type Target = $context_type;
            fn deref(&self) -> &Self::Target {
                &self.context
            }
        }
        #[OpenApi]
        impl $router_name {
            /// The auth server will post here when a user loggs in.
            #[oai(path = $url, method = "post")]
            async fn callback(&self, body: PlainText<String>) -> Result<(), CallbackResponseError> {
                assure_verification_key()
                    .await
                    .map_err(|_| CallbackResponseError::Unknown)?;
                let $data: CallbackData = AUTH_KEY
                    .with_borrow(|key| {
                        let Some(key) = key else {
                            return Err(CallbackResponseError::Unknown);
                        };
                        Ok(VALIDATION
                            .with(|validation| jsonwebtoken::decode(&**body, key, validation)))
                    })?
                    .map_err(|_| CallbackResponseError::SignatureInvalid)?
                    .claims;

                let $context = &**self;
                $cb;

                Ok(())
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use poem_openapi::OpenApiService;

    pub struct Context {
        number: u64,
    }
    auth_callback_router!(AuthRouter, Context, "/api/v0/auth", async |ctx, data| {
        println!("Data: {data:?}, context number: {}", ctx.number);
    });

    #[test]
    fn auth_callback() {
        let _api_service = OpenApiService::new(
            AuthRouter {
                context: Context { number: 1 },
            },
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        )
        .server("http://localhost:21443");
    }
}
