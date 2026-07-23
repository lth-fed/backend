use std::marker::PhantomData;

use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use ed25519_dalek::VerifyingKey;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode_header};
use minilith_errors::{
    MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _,
};
use opentelemetry::trace::TraceContextExt as _;
use poem_openapi::SecurityScheme;
use poem_openapi::auth::Bearer;
use serde::Deserialize;
use serde::de::DeserializeOwned;

pub mod callbacks;

const AUTH_KEY_PATH: &str = "/oidc/v1/certs";
#[cfg(debug_assertions)]
const AUTH_KEY_BASE: &str = "http://localhost:8001";
#[cfg(not(debug_assertions))]
const AUTH_KEY_BASE: &str = "https://auth.teknologappen.se";

/// [`jsonwebtoken`] doesn't support `EdDSA` :(
/// With `kid="main"`.
///
/// It can however decode and verify signatures.
#[must_use]
pub fn eddsa_to_jwk(key: &VerifyingKey) -> Jwk {
    // we have to do this bullshit ourselves because Jwk::from_encoding_key doesn't work!
    let compressed_point = key.to_edwards().compress().to_bytes();

    Jwk {
        common: jsonwebtoken::jwk::CommonParameters {
            key_id: Some("main".to_owned()),
            ..Default::default()
        },
        algorithm: jsonwebtoken::jwk::AlgorithmParameters::OctetKeyPair(
            jsonwebtoken::jwk::OctetKeyPairParameters {
                key_type: jsonwebtoken::jwk::OctetKeyPairType::OctetKeyPair,
                curve: jsonwebtoken::jwk::EllipticCurve::Ed25519,
                x: BASE64_URL_SAFE_NO_PAD.encode(compressed_point),
            },
        ),
    }
}

/// Trait to allow multiple [`AuthContext`]s to be attached to an endpoint.
///
/// We need both an [`AuthContext`] for the auth provider ([`AuthUrl`]) and the transactions api
/// ([`TransactionsUrl`]).
pub trait AuthContextProvider: Clone {
    /// This MUST be an url which returns a set of Json Web Keys.
    /// This MUST use HTTPS in production.
    fn url() -> String;
}

#[derive(Clone, Copy, Debug)]
pub struct AuthUrl;
impl AuthContextProvider for AuthUrl {
    fn url() -> String {
        "https://auth.teknologappen.se/api/v0/verifying-key".to_owned()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct TransactionsUrl;
impl AuthContextProvider for TransactionsUrl {
    fn url() -> String {
        "https://transactions.teknologappen.se/v0/jwks".to_owned()
    }
}

#[derive(Clone, Debug)]
#[must_use]
pub struct JwkContext<Url: Clone> {
    jwks: JwkSet,
    validation: Validation,
    #[cfg(debug_assertions)]
    testing: bool,
    phantom: PhantomData<Url>,
}
impl<Url: AuthContextProvider> JwkContext<Url> {
    /// This can not be used by `fed-auth`, since that's the service which this depends on!
    ///
    /// If audience is empty, it's not checked. Audience is your `client_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if it was not possible to get the verifying key.
    pub async fn new(audience: impl Into<String>) -> color_eyre::Result<Self> {
        let resp = match reqwest::get(format!("{AUTH_KEY_BASE}{AUTH_KEY_PATH}")).await {
            Ok(resp) => resp,
            #[allow(unused_variables, reason = "cfg")]
            Err(err) => {
                #[cfg(debug_assertions)]
                {
                    use tracing::warn;

                    warn!(
                        "Defaulting authentication, user will always be \
                        `lund-university:aa0000bb-s` \
                        (you still have to provide authorization header)"
                    );
                    return Ok(Self::from_jwks(audience, JwkSet { keys: vec![] }));
                }
                #[cfg(not(debug_assertions))]
                return Err(err.into());
            }
        };
        if resp.status() != poem::http::StatusCode::OK {
            return Err(color_eyre::eyre::Error::msg("failed getting verifying key"));
        }
        let bytes = resp.bytes().await?;
        let jwks: JwkSet = serde_json::from_slice(&bytes)?;
        Ok(Self::from_jwks(audience, jwks))
    }
    /// If audience is empty, it's not checked. Audience is your `client_id`.
    pub fn from_jwks(audience: impl Into<String>, jwks: JwkSet) -> Self {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_nbf = true;
        let aud = audience.into();
        if aud.is_empty() {
            validation.validate_aud = false;
        } else {
            validation.set_audience(&[aud]);
        }

        Self {
            #[cfg(debug_assertions)]
            testing: jwks.keys.is_empty(),
            jwks,
            validation,
            phantom: PhantomData,
        }
    }
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
}

fn decode_jwt<T: DeserializeOwned>(
    token: &str,
    context: &JwkContext<impl AuthContextProvider>,
) -> Result<TokenData<T>, MinilithEndpointError> {
    let header = decode_header(token).wrap_err_unauthorized(
        "You don't have a valid login-session. \
            Try logging out and in again or clearing cookies.",
    )?;

    let kid = header.kid.wrap_err_bad_frontend(
        "your access token has no key ID which means we cannot validate it",
    )?;

    let jwk = context
        .jwks
        .find(&kid)
        .ok_or(())
        .wrap_err_unauthorized("your access token has an invalid key ID associated with it")?;
    let key = DecodingKey::from_jwk(jwk).wrap_err_internal("internal error, jwk invalid")?;
    jsonwebtoken::decode(token, &key, &context.validation).wrap_err_unauthorized(
        "You don't have a valid login-session. \
            Try logging out and in again or clearing cookies.",
    )
}

/// Returns the logged in user. Spec says `OAuth2` but it really is OIDC but without automatic
/// discovery. Does return a string instead of the full error if the header `authorization` is not
/// defined.
///
/// [`AuthContext`] with [`AuthUrl`] MUST BE registered using [`poem::EndpointExt::data`].
///
/// # Errors
///
/// [`MinilithEndpointError`]. UK, AUTH.
#[derive(Debug, SecurityScheme)]
#[cfg_attr(
    debug_assertions,
    oai(
        ty = "oauth2",
        key_in = "header",
        key_name = "authorization",
        bearer_format = "JWT",
        flows(authorization_code(
            authorization_url = "http://localhost:8001/oidc/v1/authorize",
            token_url = "http://localhost:8001/oidc/v1/token",
        )),
        checker = "User::from_token"
    )
)]
#[cfg_attr(
    not(debug_assertions),
    oai(
        ty = "oauth2",
        key_in = "header",
        key_name = "authorization",
        bearer_format = "JWT",
        flows(authorization_code(
            authorization_url = "https://auth.teknologappen.se/oidc/v1/authorize",
            token_url = "https://auth.teknologappen.se/oidc/v1/token",
        )),
        checker = "User::from_token"
    )
)]
#[derive(Clone)]
pub struct User(String);
impl User {
    #[must_use]
    pub fn get_id(&self) -> &str {
        &self.0
    }
    #[allow(clippy::unused_async, reason = "poem_openapi wants us to")]
    async fn from_token(req: &poem::Request, token: Bearer) -> poem::Result<String> {
        let context: &JwkContext<AuthUrl> = req
            .data()
            .wrap_err_internal("AuthContext not registered as data!")?;
        let cx = opentelemetry::Context::current();
        let span = cx.span();
        if context.testing {
            span.set_attribute(opentelemetry::KeyValue::new(
                opentelemetry_semantic_conventions::attribute::USER_ID,
                "lund-university:aa0000bb-s",
            ));
            return Ok("lund-university:aa0000bb-s".to_owned());
        }

        let data = decode_jwt::<Claims>(&token.token, context)?;
        span.set_attribute(opentelemetry::KeyValue::new(
            opentelemetry_semantic_conventions::attribute::USER_ID,
            data.claims.sub.clone(),
        ));
        Ok(data.claims.sub)
    }
}
