//! This `OpenID Connect` implementation supports only the Authorization Code Flow. It also uses PKCE
//! and JWTs for access codes.
//!
//! The structure is a bit odd, because we are both an `OpenID Provider` and a sort of "select one
//! login method" provider. This means we have several internal providers. The internal providers'
//! code is located in `./api.rs` instead.

use std::fmt::{Debug, Display};
use std::ops::Deref;

use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use fed_auth_verifier::User;
use jsonwebtoken::jwk::JwkSet;
use poem::http::{StatusCode, Uri};
use poem_openapi::payload::{Form, Json, PlainText, Response};
use poem_openapi::types::ToJSON as _;
use poem_openapi::{ApiResponse, Enum, Object, OpenApi};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use sqlx::types::time::OffsetDateTime;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::context::CallbackUrl;
use crate::{API_DOMAIN, Context, WEBSITE_DOMAIN, context, jwt, random_id};

const TEKNOLOGAPPEN_ALLOWED_DOMAINS: &[&str] = &[
    "https://teknologappen.se",
    "https://api.teknologappen.se",
    // ios app
    "capacitor://localhost",
    // android app
    "https://localhost",
];
/// `(client_id, allowed_domains[])[]`.
const ALLOWED_DOMAINS: &[(&str, &[&str])] = &[
    ("teknologappen", TEKNOLOGAPPEN_ALLOWED_DOMAINS),
    ("esek", &["https://auth.esek.se"]),
    ("fsek", &["https://fsektionen.se"]),
    ("dsek", &["https://auth.dsek.se", "https://dsek.se"]),
];
fn eq_uri_domain(uri: &Uri, domain: &str) -> bool {
    let (scheme, authority) = domain.split_once("://").unwrap_or(("", ""));
    uri.scheme_str() == Some(scheme) && uri.authority().is_some_and(|auth| auth == authority)
}
fn is_allowed_domain(client_id: &str, domain: &Uri) -> bool {
    #[cfg(debug_assertions)]
    if eq_uri_domain(domain, "http://localhost:5173")
        || eq_uri_domain(domain, "http://localhost:8000")
        || eq_uri_domain(domain, API_DOMAIN)
        || eq_uri_domain(domain, WEBSITE_DOMAIN)
    {
        return true;
    }
    ALLOWED_DOMAINS.iter().any(|(cid, allowed)| {
        *cid == client_id && allowed.iter().any(|allowed| eq_uri_domain(domain, allowed))
    })
}
fn is_teknologappen_domain(domain: &Uri) -> bool {
    #[cfg(debug_assertions)]
    if eq_uri_domain(domain, "http://localhost:5173")
        || eq_uri_domain(domain, "http://localhost:8000")
        || eq_uri_domain(domain, API_DOMAIN)
        || eq_uri_domain(domain, WEBSITE_DOMAIN)
    {
        return true;
    }
    TEKNOLOGAPPEN_ALLOWED_DOMAINS
        .iter()
        .any(|allowed| eq_uri_domain(domain, allowed))
}
pub const ACCESS_TOKEN_VALID_FOR: u64 = 15 * 60;
pub const CALLBACK_TOKEN_VALID_FOR: u64 = 60;

#[derive(Enum, Clone, PartialEq, Eq)]
#[oai(rename_all = "snake_case")]
enum OAuth2ErrorKind {
    InvalidRequest,
    InvalidClient,
    InvalidGrant,
    UnauthorizedClient,
    UnsupportedGrantType,
    InvalidScope,
    // Authorize endpoint
    InteractionRequired,
    RequestNotSupported,
    RequestUriNotSupported,
    RegistrationNotSupported,

    // Custom
    Internal,
}
#[derive(Object, Clone)]
struct OAuth2Error {
    error: OAuth2ErrorKind,
    error_description: String,
}
#[derive(ApiResponse)]
#[oai(bad_request_handler = "bad_request_handler")]
enum OAuth2ApiResponse {
    #[oai(status = 400)]
    OAuth2Error(Json<OAuth2Error>),
    #[oai(status = 500)]
    Internal,
}
#[allow(clippy::needless_pass_by_value, reason = "poem wants us to")]
fn bad_request_handler(err: poem::Error) -> OAuth2ApiResponse {
    OAuth2ApiResponse::oauth2error(OAuth2ErrorKind::InvalidRequest, err.to_string())
}
impl OAuth2ApiResponse {
    fn oauth2error(kind: OAuth2ErrorKind, description: impl Into<String>) -> Self {
        Self::OAuth2Error(Json(OAuth2Error {
            error: kind,
            error_description: description.into(),
        }))
    }
    #[track_caller]
    fn db(err: impl Display) -> Self {
        error!("Database connection failed: {err}");
        Self::Internal
    }
    fn grant_type() -> Self {
        Self::oauth2error(
            OAuth2ErrorKind::InvalidGrant,
            "grant has to be either refresh_token or authorization_code. \
                check you have all fields",
        )
    }
    fn unauth(message: impl Into<String>) -> Self {
        Self::oauth2error(OAuth2ErrorKind::UnauthorizedClient, message)
    }
}
type OAuth2Result<T> = Result<T, OAuth2ApiResponse>;

#[derive(Serialize)]
struct IdTokenClaims {
    iss: String,
    sub: String,
    auth_time: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
}

#[derive(Object, Clone, Deserialize)]
struct TokenAuthoriationBody {
    #[oai(validator(pattern = "authorization_code"))]
    grant_type: String,
    code: String,
    redirect_uri: String,
    client_id: String,

    // PKCE
    code_verifier: String,
}
impl TryFrom<&TokenBody> for TokenAuthoriationBody {
    type Error = ();
    fn try_from(value: &TokenBody) -> Result<Self, Self::Error> {
        if value.grant_type != "authorization_code" {
            return Err(());
        }
        Ok(Self {
            grant_type: value.grant_type.clone(),
            code: value.code.clone().ok_or(())?,
            redirect_uri: value.redirect_uri.clone().ok_or(())?,
            client_id: value.client_id.clone().ok_or(())?,
            code_verifier: value.code_verifier.clone().ok_or(())?,
        })
    }
}
#[derive(Object, Clone, Deserialize)]
struct TokenRefreshBody {
    #[oai(validator(pattern = "refresh_token"))]
    grant_type: String,
    // we only give out Uuids
    refresh_token: Uuid,
    // scope
}
impl TryFrom<&TokenBody> for TokenRefreshBody {
    type Error = ();
    fn try_from(value: &TokenBody) -> Result<Self, Self::Error> {
        if value.grant_type != "refresh_token" {
            return Err(());
        }
        Ok(Self {
            grant_type: value.grant_type.clone(),
            refresh_token: value.refresh_token.ok_or(())?,
        })
    }
}
#[derive(Object, Clone, Deserialize)]
struct TokenBody {
    grant_type: String,

    // grant_type = authorization
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    // PKCE
    code_verifier: Option<String>,

    // grant_type = refresh
    refresh_token: Option<Uuid>,
    // scope
}
#[derive(Object)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    refresh_token: Uuid,
    id_token: String,
}

#[derive(Object, Deserialize, Serialize)]
struct AuthorizeBody {
    // openid & potentially others
    scope: String,
    // code
    response_type: String,
    client_id: String,
    redirect_uri: String,
    state: Option<String>,
    nonce: Option<String>,
    prompt: Option<String>,
    // max_age: we always auth
    request: Option<String>,
    request_uri: Option<String>,
    registration: Option<String>,

    // PKCE
    code_challenge: String,
    code_challenge_method: String,

    // custom
    // space separated list of providers allowed
    providers: Option<String>,
    #[serde(flatten)]
    server_callback: Option<CallbackUrl>,
}
#[derive(ApiResponse)]
#[oai(bad_request_handler = "bad_request_handler_authorize")]
enum AuthorizeResponse {
    #[oai(status = 200)]
    DatasharingOk(Json<DatasharingResponse>),
    #[oai(status = 302)]
    Redirect(PlainText<String>),
    #[oai(status = 400)]
    RedirectUriError(Json<OAuth2Error>),
}
impl AuthorizeResponse {
    fn redirect() -> Self {
        Self::Redirect(PlainText(String::new()))
    }
}
#[allow(clippy::needless_pass_by_value, reason = "poem wants us to")]
fn bad_request_handler_authorize(err: poem::Error) -> AuthorizeResponse {
    OAuth2ApiResponse::oauth2error(OAuth2ErrorKind::InvalidRequest, err.to_string()).into()
}
impl From<OAuth2ApiResponse> for AuthorizeResponse {
    fn from(value: OAuth2ApiResponse) -> Self {
        AuthorizeResponse::RedirectUriError(match value {
            OAuth2ApiResponse::OAuth2Error(err) => err,
            OAuth2ApiResponse::Internal => Json(OAuth2Error {
                error: OAuth2ErrorKind::Internal,
                error_description: String::new(),
            }),
        })
    }
}
impl From<OAuth2ApiResponse> for Response<AuthorizeResponse> {
    fn from(value: OAuth2ApiResponse) -> Self {
        Response::new(value.into())
    }
}
struct OAuth2ErrorCtx<'a>(&'a String, &'a Context, &'static str);
#[allow(clippy::needless_pass_by_value, reason = "poem wants us to")]
fn oauth2error_redirect(
    kind: OAuth2ErrorKind,
    description: impl AsRef<str>,
    ctx: &OAuth2ErrorCtx,
) -> Response<AuthorizeResponse> {
    if kind == OAuth2ErrorKind::Internal {
        use opentelemetry::KeyValue;
        use opentelemetry_semantic_conventions::trace;

        let labels = vec![
            KeyValue::new(trace::URL_FULL, ctx.2),
            KeyValue::new(trace::HTTP_RESPONSE_STATUS_CODE, 500i64),
            KeyValue::new(trace::EXCEPTION_MESSAGE, description.as_ref().to_owned()),
        ];
        // there will be some double-counting but I'm fine with it, we really just worry about the
        // 500s
        ctx.1.request_counter.add(1, &labels);
        ctx.1.error_counter.add(1, &labels);
    }
    Response::new(AuthorizeResponse::Redirect(PlainText(String::new())))
        .status(StatusCode::FOUND)
        .header(
            "location",
            format!(
                "{}{}error={}&error_description={}",
                ctx.0,
                if ctx.0.contains('?') { '&' } else { '?' },
                kind.to_json_string(),
                description.as_ref()
            ),
        )
}

#[derive(Object)]
struct DatasharingRequest {
    accepted: bool,
    code: String,
}
#[derive(Object, Clone)]
struct DatasharingResponse {
    url: String,
}

#[derive(Clone)]
pub(crate) struct MainRouter {
    pub context: Context,
}
impl Deref for MainRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
#[OpenApi]
impl MainRouter {
    /// Returns the JWKs.
    ///
    /// TODO: make this rotate keys.
    #[oai(path = "/certs", method = "get")]
    async fn certs(&self) -> Response<Json<poem_openapi::types::Any<&JwkSet>>> {
        Response::new(Json(poem_openapi::types::Any(&self.jwks)))
    }
    /// Get JWT access token and a new refresh token.
    ///
    /// Same parameters as POST version, but in query instead of body. Refer to it for API
    /// specification.
    #[oai(path = "/token", method = "get")]
    async fn token_get(
        &self,
        refresh: Option<poem::web::Query<TokenRefreshBody>>,
        authorization: Option<poem::web::Query<TokenAuthoriationBody>>,
    ) -> OAuth2Result<Json<TokenResponse>> {
        match (refresh, authorization) {
            (Some(refresh), _) => self.handle_refresh(refresh.0).await,
            (None, Some(auth)) => self.handle_authorize_token(auth.0).await,
            (None, None) => Err(OAuth2ApiResponse::oauth2error(
                OAuth2ErrorKind::InvalidRequest,
                "didn't match either refresh or authorization_code grant types",
            )),
        }
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/token", method = "post")]
    async fn token(&self, body: Form<TokenBody>) -> OAuth2Result<Json<TokenResponse>> {
        if let Ok(refresh) = TokenRefreshBody::try_from(&body.0) {
            self.handle_refresh(refresh).await
        } else if let Ok(auth) = TokenAuthoriationBody::try_from(&body.0) {
            self.handle_authorize_token(auth).await
        } else {
            Err(OAuth2ApiResponse::grant_type())
        }
    }
    async fn handle_refresh(&self, refresh: TokenRefreshBody) -> OAuth2Result<Json<TokenResponse>> {
        let mut conn = self.db.begin().await.map_err(OAuth2ApiResponse::db)?;

        let removed = sqlx::query!(
            "delete from auth_refresh_tokens where refresh_token = $1
                    returning client_id, user_id, auth_time, nonce",
            refresh.refresh_token,
        )
        .fetch_optional(&mut conn.executor())
        .await
        .map_err(OAuth2ApiResponse::db)?;

        let Some(row) = removed else {
            return Err(OAuth2ApiResponse::unauth("invalid refresh_token"));
        };
        let user_id = row.user_id;

        let new_refresh = Uuid::new_v4();
        sqlx::query!(
            "insert into auth_refresh_tokens
                (refresh_token, client_id, user_id, auth_time, nonce)
            values ($1, $2, $3, $4, $5)",
            new_refresh,
            row.client_id,
            user_id,
            row.auth_time,
            None::<String>,
        )
        .execute(&mut conn.executor())
        .await
        .map_err(OAuth2ApiResponse::db)?;

        conn.commit().await.map_err(OAuth2ApiResponse::db)?;

        let claims = jwt::StandardClaims::new(
            &row.client_id,
            ACCESS_TOKEN_VALID_FOR,
            jwt::AccesTokenClaims {
                sub: user_id.clone(),
            },
        );
        let access_token =
            jwt::encode(&claims, &self.private_key).map_err(|_| OAuth2ApiResponse::Internal)?;

        #[allow(clippy::cast_sign_loss, reason = "we are taking the abs before!")]
        let id_token = jwt::StandardClaims::new(
            row.client_id,
            ACCESS_TOKEN_VALID_FOR,
            IdTokenClaims {
                iss: WEBSITE_DOMAIN.to_owned(),
                sub: user_id,
                auth_time: (row.auth_time - OffsetDateTime::UNIX_EPOCH)
                    .whole_seconds()
                    .wrapping_abs() as u64,
                nonce: row.nonce,
            },
        );

        Ok(Json(TokenResponse {
            access_token,
            refresh_token: new_refresh,
            token_type: "bearer".to_owned(),
            expires_in: ACCESS_TOKEN_VALID_FOR,
            id_token: jwt::encode(&id_token, &self.private_key)
                .map_err(|_| OAuth2ApiResponse::Internal)?,
        }))
    }
    #[allow(
        clippy::too_many_lines,
        reason = "it's a linear function and it makes sense, extracting sql
        calls would not make it much easier to read"
    )]
    async fn handle_authorize_token(
        &self,
        mut auth: TokenAuthoriationBody,
    ) -> OAuth2Result<Json<TokenResponse>> {
        let Some(session) = self.auth_sessions.get(&auth.code) else {
            return Err(OAuth2ApiResponse::unauth("invalid code"));
        };
        if session.redirect_uri != auth.redirect_uri {
            return Err(OAuth2ApiResponse::oauth2error(
                OAuth2ErrorKind::InvalidRequest,
                "redirect_uri must match",
            ));
        }
        if session.client_id != auth.client_id {
            return Err(OAuth2ApiResponse::oauth2error(
                OAuth2ErrorKind::InvalidRequest,
                "client_id must match",
            ));
        }
        let mut hash = sha2::Sha256::new();
        hash.update(&auth.code_verifier);
        auth.code_verifier.clear();
        BASE64_URL_SAFE_NO_PAD.encode_string(hash.finalize(), &mut auth.code_verifier);
        if session.code_challenge != auth.code_verifier {
            return Err(OAuth2ApiResponse::oauth2error(
                OAuth2ErrorKind::InvalidGrant,
                "the PKCE validation failed",
            ));
        }
        if !session.datasharing_confirmed && session.redirect_requires_datasharing {
            return Err(OAuth2ApiResponse::unauth(
                "skipped parts of the auth process",
            ));
        }
        let user_data = session
            .validated_user
            .as_ref()
            .ok_or_else(|| OAuth2ApiResponse::unauth("skipped parts of the auth process"))?;

        if let Some(cb_url) = &session.callback {
            let token = jwt::encode(
                &jwt::StandardClaims::new(&session.client_id, CALLBACK_TOKEN_VALID_FOR, user_data),
                &self.private_key,
            )
            .map_err(|_| OAuth2ApiResponse::Internal)?;
            match cb_url.as_latest() {
                context::CallbackUrlVersion::V1 { url } => {
                    self.reqwest_client
                        .post(url)
                        .body(token)
                        .send()
                        .await
                        .inspect_err(|err| error!("auth callback POST failed: {err}"))
                        .map_err(|_| OAuth2ApiResponse::Internal)?;
                }
            }
        }

        let refresh_token = Uuid::new_v4();
        let row = sqlx::query!(
            "insert into auth_refresh_tokens
                (refresh_token, client_id, user_id, nonce, auth_time)
                values ($1, $2, $3, $4, now())
            returning auth_time",
            refresh_token,
            session.client_id,
            user_data.sub,
            session.nonce,
        )
        .fetch_one(&self.db)
        .await
        .inspect_err(|err| error!("Error inserting refresh token into DB: {err}"))
        .map_err(|_| OAuth2ApiResponse::Internal)?;

        self.auth_sessions.invalidate(&auth.code);

        #[allow(clippy::cast_sign_loss, reason = "we are taking the abs before!")]
        let id_token = jwt::StandardClaims::new(
            &session.client_id,
            ACCESS_TOKEN_VALID_FOR,
            IdTokenClaims {
                iss: WEBSITE_DOMAIN.to_owned(),
                sub: user_data.sub.clone(),
                auth_time: (row.auth_time - OffsetDateTime::UNIX_EPOCH)
                    .whole_seconds()
                    .wrapping_abs() as u64,
                nonce: session.nonce,
            },
        );

        let claims = jwt::StandardClaims::new(
            session.client_id,
            ACCESS_TOKEN_VALID_FOR,
            jwt::AccesTokenClaims {
                sub: user_data.sub.clone(),
            },
        );
        let access_token =
            jwt::encode(&claims, &self.private_key).map_err(|_| OAuth2ApiResponse::Internal)?;

        Ok(Json(TokenResponse {
            access_token,
            refresh_token,
            token_type: "bearer".to_owned(),
            expires_in: ACCESS_TOKEN_VALID_FOR,
            id_token: jwt::encode(&id_token, &self.private_key)
                .map_err(|_| OAuth2ApiResponse::Internal)?,
        }))
    }

    #[oai(path = "/userinfo", method = "post", method = "get")]
    async fn userinfo_post(&self, user: User) -> Json<jwt::UserInfoClaims> {
        Json(jwt::UserInfoClaims {
            sub: user.get_id().to_owned(),
        })
    }
    fn redirect_provider(
        &self,
        body: AuthorizeBody,
        code: String,
        url: String,
        redirect_uri: &Uri,
    ) -> Response<AuthorizeResponse> {
        // extension of params: provider
        let session = context::AuthSession {
            redirect_uri: body.redirect_uri,
            client_id: body.client_id,
            state: body.state,
            nonce: body.nonce,
            callback: body.server_callback,
            code_challenge: body.code_challenge,

            validated_user: None,
            datasharing_confirmed: false,
            redirect_requires_datasharing: !is_teknologappen_domain(redirect_uri),
        };
        self.auth_sessions.insert(code, session);

        Response::new(AuthorizeResponse::redirect())
            .status(StatusCode::FOUND)
            .header("location", url)
    }
    /// Same parameters as POST version, but in query instead of body. Refer to it for API
    /// specification.
    #[oai(path = "/authorize", method = "get")]
    async fn authorize_get(
        &self,
        params: poem::web::Query<AuthorizeBody>,
    ) -> Response<AuthorizeResponse> {
        self.authorize(Form(params.0)).await
    }
    #[allow(
        clippy::too_many_lines,
        reason = "the logic is linear and there's just many checks and error logging"
    )]
    #[oai(path = "/authorize", method = "post")]
    async fn authorize(&self, body: Form<AuthorizeBody>) -> Response<AuthorizeResponse> {
        let Ok(redirect_uri) = body.redirect_uri.parse::<Uri>() else {
            error!(
                client_id = body.client_id,
                redirect_uri = body.redirect_uri,
                "invalid redirect_uri",
            );
            return OAuth2ApiResponse::oauth2error(
                OAuth2ErrorKind::InvalidRequest,
                "invalid redirect_uri",
            )
            .into();
        };
        if redirect_uri.scheme().is_none() || redirect_uri.authority().is_none() {
            error!(
                client_id = body.client_id,
                redirect_uri = body.redirect_uri,
                "invalid redirect_uri",
            );
            return OAuth2ApiResponse::oauth2error(
                OAuth2ErrorKind::InvalidRequest,
                "invalid redirect_uri",
            )
            .into();
        }
        let ru = &body.redirect_uri;
        let ctx = OAuth2ErrorCtx(ru, self, "/oidc/v1/authorize");

        if body.request.is_some() {
            return oauth2error_redirect(OAuth2ErrorKind::RequestNotSupported, "", &ctx);
        }
        if body.request_uri.is_some() {
            return oauth2error_redirect(OAuth2ErrorKind::RequestUriNotSupported, "", &ctx);
        }
        if body.registration.is_some() {
            return oauth2error_redirect(OAuth2ErrorKind::RegistrationNotSupported, "", &ctx);
        }
        if body.code_challenge_method != "S256" {
            return oauth2error_redirect(
                OAuth2ErrorKind::InvalidRequest,
                "code_challenge_method has to exist and be S256",
                &ctx,
            );
        }

        // default value ensures we're redirected to provider selection if no value is specified
        let mut providers = body.providers.as_deref().unwrap_or("lu mail").split(' ');
        let first_provider = providers.next().unwrap_or("non-existant-provider");
        // if there is more than 1
        if providers.next().is_some() {
            let Ok(params) = serde_urlencoded::to_string(&body.0) else {
                error!(client_id = body.client_id, "body not serializable");
                return oauth2error_redirect(
                    OAuth2ErrorKind::InvalidRequest,
                    "some part of the body was not serializable",
                    &ctx,
                );
            };
            // this redirects back with one specified provider
            return Response::new(AuthorizeResponse::redirect())
                .status(StatusCode::FOUND)
                .header("location", format!("{WEBSITE_DOMAIN}/providers/?{params}"));
        }
        let provider = first_provider;

        if !body.scope.split(' ').any(|scope| scope == "openid") {
            error!(client_id = body.client_id, "scope ! contains openid",);
            return oauth2error_redirect(
                OAuth2ErrorKind::InvalidScope,
                "scope has to contain openid",
                &ctx,
            );
        }
        if body.response_type != "code" {
            error!(
                client_id = body.client_id,
                response_type = body.response_type,
                "response_type != code",
            );
            return oauth2error_redirect(
                OAuth2ErrorKind::InvalidRequest,
                "only response_type='code' is supported",
                &ctx,
            );
        }

        if !is_allowed_domain(&body.client_id, &redirect_uri) {
            error!(
                client_id = body.client_id,
                redirect_uri = ru,
                "client_id is not allowed to redirect to this uri",
            );
            return oauth2error_redirect(
                OAuth2ErrorKind::InvalidClient,
                "client_id is not allowed to redirect to this uri",
                &ctx,
            );
        }
        if let Some(cb_url) = &body.server_callback {
            let url = cb_url.as_latest();
            let Ok(cb_url) = url.url().parse::<Uri>() else {
                error!(
                    client_id = body.client_id,
                    server_callback = ?body.server_callback,
                    "invalid server_callback",
                );
                return oauth2error_redirect(
                    OAuth2ErrorKind::InvalidRequest,
                    "invalid server_callback",
                    &ctx,
                );
            };
            if !is_allowed_domain(&body.client_id, &cb_url) {
                error!(
                    client_id = body.client_id,
                    server_callback = ?body.server_callback,
                    "client_id is not allowed to have a callback to this server",
                );
                return oauth2error_redirect(
                    OAuth2ErrorKind::InvalidClient,
                    "client_id is not allowed to have a callback to this server",
                    &ctx,
                );
            }
        }

        let (code, redirect) = match self.get_provider(provider, &ctx) {
            Ok(value) => value,
            Err(err) => return err,
        };

        self.redirect_provider(body.0, code, redirect, &redirect_uri)
    }
    #[allow(clippy::result_large_err, reason = "poem requires this type")]
    fn get_provider(
        &self,
        provider: &str,
        ctx: &OAuth2ErrorCtx,
    ) -> Result<(String, String), Response<AuthorizeResponse>> {
        Ok(match provider {
            "lu" => {
                let req = self
                    .service_provider
                    // .make_authentication_request("https://testidpv4.lu.se/idp/profile/SAML2/Redirect/SSO")
                    .make_authentication_request("https://mocksaml.com/api/saml/sso")
                    .map_err(|err| {
                        error!(?err, "Failed to make LU SSO request");
                        oauth2error_redirect(OAuth2ErrorKind::Internal, "lu sso saml2 error", ctx)
                    })?;
                let redirect = req
                    .signed_redirect("", &self.saml_private_key)
                    .ok()
                    .flatten()
                    .ok_or_else(|| {
                        error!("Failed to create LU SSO link");
                        oauth2error_redirect(OAuth2ErrorKind::Internal, "lu sso saml2 error", ctx)
                    })?;
                self.saml2_request_id_cache.insert(req.id.clone(), ());
                debug!("Added ID {} to saml2 request id cache", req.id);
                (req.id, redirect.to_string())
            }
            "email" => {
                let code = random_id();
                let redirect = format!("{WEBSITE_DOMAIN}/providers/email/?code={code}");
                (code, redirect)
            }
            "test" => {
                let code = random_id();
                let redirect = format!("{WEBSITE_DOMAIN}/providers/test/?code={code}");
                (code, redirect)
            }
            _ => {
                return Err(oauth2error_redirect(
                    OAuth2ErrorKind::InvalidRequest,
                    "provider not found",
                    ctx,
                ));
            }
        })
    }

    /// Our own custom step to confirm datasharing which we are required to do by SWAMID.
    #[oai(path = "/confirm-datasharing", method = "post")]
    async fn confirm_datasharing(
        &self,
        body: Json<DatasharingRequest>,
        headers: &poem::http::HeaderMap,
    ) -> Response<AuthorizeResponse> {
        let Some(mut session) = self.auth_sessions.get(&body.code) else {
            warn!(
                "Tried to confirm datasharing with a code which is not in the database ({})",
                body.code
            );
            return OAuth2ApiResponse::oauth2error(
                OAuth2ErrorKind::InvalidRequest,
                "no such code, try logging in from the start again",
            )
            .into();
        };
        let ru = &session.redirect_uri;
        let ctx = OAuth2ErrorCtx(ru, self, "/oidc/v1/confirm-datasharing");
        if headers
            .get("origin")
            .is_some_and(|origin| origin != WEBSITE_DOMAIN)
        {
            return oauth2error_redirect(
                OAuth2ErrorKind::InvalidRequest,
                "you must use the website to use this api",
                &ctx,
            );
        }
        if !body.accepted {
            return oauth2error_redirect(
                OAuth2ErrorKind::UnauthorizedClient,
                "user did not agree to datasharing",
                &ctx,
            );
        }
        if session.validated_user.is_none() {
            warn!(
                "Tried to confirm datasharing for a request which was not validated ({})",
                body.code
            );
            return oauth2error_redirect(
                OAuth2ErrorKind::InvalidRequest,
                "you are not authenticated, try logging in from the start again",
                &ctx,
            );
        }
        session.datasharing_confirmed = true;
        self.auth_sessions
            .insert(body.code.clone(), session.clone());

        Response::new(AuthorizeResponse::DatasharingOk(Json(
            DatasharingResponse {
                url: session.provider_callback_next_url(&body.code),
            },
        )))
    }
}
