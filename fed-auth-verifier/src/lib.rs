use jsonwebtoken::{Algorithm, Validation};
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _};
use poem::http::StatusCode;
use poem_openapi::auth::Bearer;
use poem_openapi::{ApiExtractor, Object, SecurityScheme};
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
            MinilithEndpointError::internal_error("AuthContext not registered as data!")
        })?;

        let data =
            jsonwebtoken::decode::<Claims>(&token.token, &context.auth_key, &context.validation)
                .wrap_err_unauthorized("decode failed")?;
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
            .wrap_err_bad_user("invalid utf8 body", "<body>")?;
        let data: CallbackDataV1 =
            jsonwebtoken::decode(&body, &context.auth_key, &context.validation)
                .wrap_err_unauthorized("jwt invalid on decode")?
                .claims;
        Ok(data)
    }
}
