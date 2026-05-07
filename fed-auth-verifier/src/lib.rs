use std::cell::{LazyCell, RefCell};

use jsonwebtoken::{Algorithm, Validation};
use poem::FromRequest;
use poem::http::StatusCode;
use serde::Deserialize;

const AUTH_KEY_URL: &str = "https://auth.teknologappen.se/api/verify-key.der";

thread_local! {
    static AUTH_KEY: RefCell<Option<jsonwebtoken::DecodingKey>> = const { RefCell::new(None) };
    /// So we don't have to re-allocate for every check
    static VALIDATION: LazyCell<Validation> = LazyCell::new(|| {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_nbf = true;
        validation.set_audience(&["teknologappen-auth"]);
        validation
    });
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
        if AUTH_KEY.with_borrow(Option::is_none) {
            let resp = reqwest::get(AUTH_KEY_URL)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            if resp.status() != StatusCode::OK {
                return Err(StatusCode::INTERNAL_SERVER_ERROR.into());
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
