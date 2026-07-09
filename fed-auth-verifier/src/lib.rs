use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use ed25519_dalek::VerifyingKey;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode_header};
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _};
use opentelemetry::trace::TraceContextExt as _;
use poem_openapi::auth::Bearer;
use poem_openapi::{ApiExtractor, Object, SecurityScheme};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tracing::error;

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

#[derive(Clone, Debug)]
#[must_use]
pub struct AuthContext {
    jwks: JwkSet,
    validation: Validation,
    #[cfg(debug_assertions)]
    testing: bool,
}
impl AuthContext {
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
        }
    }
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
}

fn decode_jwt<T: DeserializeOwned>(
    token: &str,
    context: &AuthContext,
) -> Result<TokenData<T>, MinilithEndpointError> {
    let header = decode_header(token).map_err(|error| {
        MinilithEndpointError::unauthorized(
            "You don't have a valid login-session. \
            Try logging out and in again or clearing cookies.",
            error,
        )
    })?;

    let Some(kid) = header.kid else {
        return Err(MinilithEndpointError::bad_frontend_code(
            "your access token has no key ID which means we cannot validate it",
            "",
        ));
    };

    let Some(jwk) = context.jwks.find(&kid) else {
        return Err(MinilithEndpointError::unauthorized(
            "ACCESS_KID_INVALID",
            "your access token has an invalid key ID associated with it",
        ));
    };
    let Ok(key) = DecodingKey::from_jwk(jwk) else {
        return Err(MinilithEndpointError::internal_error(
            "internal error, jwk invalid",
        ));
    };
    jsonwebtoken::decode(token, &key, &context.validation).map_err(|error| {
        MinilithEndpointError::unauthorized(
            "You don't have a valid login-session. \
            Try logging out and in again or clearing cookies.",
            error,
        )
    })
}

/// Returns the logged in user. Spec says `OAuth2` but it really is OIDC but without automatic
/// discovery. Does return a string instead of the full error if the header `authorization` is not
/// defined.
///
/// [`AuthContext`] MUST BE registered using [`poem::EndpointExt::data`].
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
        let context: &AuthContext = req.data().ok_or_else(|| {
            MinilithEndpointError::internal_error("AuthContext not registered as data!")
        })?;
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
impl<'a> ApiExtractor<'a> for CallbackDataV1 {
    // to make the OpenApi look pretty
    // inspired by the impl of PlainText<String>
    type ParamType = ();
    type ParamRawType = ();

    const TYPES: &'static [poem_openapi::ApiExtractorType] =
        &[poem_openapi::ApiExtractorType::RequestObject];
    const PARAM_IS_REQUIRED: bool = false;

    fn request_meta() -> Option<poem_openapi::registry::MetaRequest> {
        let mut jwt = poem_openapi::registry::MetaSchema::new("string");
        jwt.example = Some(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
            eyJsb2dnZWRJbkFzIjoiYWRtaW4iLCJpYXQiOjE0MjI3Nzk2Mzh9.\
            gzSraSYS8EXBxLN_oWnFSRgCzcmJmMjLiuyu5CSpyHI"
                .into(),
        );

        Some(poem_openapi::registry::MetaRequest {
            description: None,
            content: vec![poem_openapi::registry::MetaMediaType {
                content_type: "application/jwt",
                schema: poem_openapi::registry::MetaSchemaRef::Inline(Box::new(jwt)),
            }],
            required: true,
        })
    }

    async fn from_request(
        request: &'a poem::Request,
        body: &mut poem::RequestBody,
        _param_opts: poem_openapi::ExtractParamOptions<()>,
    ) -> poem::Result<Self> {
        let context: &AuthContext = request.data().ok_or_else(|| {
            MinilithEndpointError::internal_error("AuthContext not registered as data!")
        })?;
        let body = body.take().map_err(|err| {
            error!(?err, "Somebody took our body!");
            MinilithEndpointError::internal_error("")
        })?;
        let body = body
            .into_string()
            .await
            .wrap_err_bad_user("AUTH_CB_BDY_UTF8", "<body>")?;
        let data = decode_jwt::<CallbackDataV1>(&body, context)?;
        Ok(data.claims)
    }
}
