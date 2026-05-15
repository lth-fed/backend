//! - [x] tappen hemsidan: användare vill logga in med auth
//! - [x] skickar till providers/lu med body av continue url & callback (put that & save origin header in cache)
//!   `curl -d '{ "continue_url": "https://icelk.dev?wow" }' https://auth.teknologappen.se/api/v0/providers/lu -H 'origin: icelk.dev' -H "content-type: application/json"`
//!   gå till hemsidan!
//!   - [x] origin & callback host must match
//! - [x] login sker
//! - [x] auth får tillbaka token
//! - [x] auth sparar (token, origin, continue, callback) ett tag med ett ID
//! - visar en sida för användaren om hur den vill dela sina uppgifter (redirect från post sidan med ?id=...)
//!   `curl -d '{ "accepted": true, "id": "<id>" }' https://auth.teknologappen.se/api/v0/confirm-datasharing -H "content-type: application/json`
//! - om nej, redirect back / postMessage, no ID
//! - [x] om ja, make request set http only cookie & callback to server, it returns redirect url & status redirect back / postMessage
//!   `curl -vd '{ "domain": "icelk.dev", "refresh_token": "<...>" }' https://auth.teknologappen.se/api/v0/refresh -H "content-type: application/json"`
#![allow(clippy::unused_async, reason = "OpenAPI requires async handlers")]
#![allow(
    missing_debug_implementations,
    reason = "we can't add debug to e.g. Context"
)]
#![allow(clippy::same_name_method, reason = "rust_embed uses it")]
use std::collections::HashMap;
use std::io::Cursor;
use std::ops::Deref;
use std::path::PathBuf;

use fed_auth_verifier::User;
use lettre::AsyncTransport as _;
use mini_moka::sync::Cache;
use poem::EndpointExt as _;
use poem::endpoint::EmbeddedFilesEndpoint;
use poem::http::Method;
use poem::middleware::{CookieJarManager, Cors};
use poem::web::cookie::{Cookie, CookieJar, SameSite};
use reqwest::StatusCode;
use samael::metadata::{
    AttributeConsumingService, ContactPerson, ContactType, EntityDescriptor, LocalizedName,
    LocalizedUri, RequestedAttribute,
};
use samael::service_provider::{ServiceProvider, ServiceProviderBuilder};

use base64::Engine as _;
use color_eyre::{Section as _, eyre::Context as _};
use ed25519_dalek::pkcs8::DecodePrivateKey as _;
use jsonwebtoken::EncodingKey;
use poem::{Route, Server, listener::TcpListener};
use poem_openapi::payload::{Binary, Form, Json, PlainText, Response};
use poem_openapi::{ApiResponse, Object, OpenApi, OpenApiService};
use samael::traits::ToXml as _;
use serde::Serialize;
use sqlx::migrate::MigrateDatabase as _;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres};
use tracing::{debug, error, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use uuid::Uuid;
use xmltree::XMLNode;

#[derive(rust_embed::Embed)]
#[folder = "../../frontend/auth/build"]
struct Website;
/// We need the frontend to be built!
const _INDEX: &str = include_str!("../../../frontend/auth/build/index.html");

const REFRESH_TOKEN_COOKIE: &str = "teknologappen-auth-refresh-token";
const ALLOWED_DOMAINS: &[&str] = &[
    "https://teknologappen.se",
    "https://auth.esek.se",
    "https://fsektionen.se",
    "https://auth.dsek.se",
];

fn random_id() -> String {
    // hex-formatted random string
    format!("{:X}", rand::random::<u128>())
}

#[derive(Clone, Debug)]
struct AuthData {
    origin: String,
    callback_url: Option<String>,
    continue_url: String,

    validated_user: Option<UserData>,
}
#[derive(Serialize, Clone, Debug)]
struct JwtData<T> {
    exp: u64,
    nbf: u64,
    aud: String,
    #[serde(flatten)]
    other_claims: T,
}
impl<T> JwtData<T> {
    pub fn new(claims: T) -> Self {
        let now = jsonwebtoken::get_current_timestamp();
        Self {
            exp: now + 60 * 60,
            nbf: now,
            aud: "teknologappen.se".into(),
            other_claims: claims,
        }
    }
}

#[derive(Clone)]
struct Context {
    db: PgPool,
    reqwest_client: reqwest::Client,
    email: Option<(
        lettre::Address,
        lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    )>,
    private_key: EncodingKey,
    public_key: Vec<u8>,
    service_provider: ServiceProvider,
    saml_private_key: openssl::pkey::PKey<openssl::pkey::Private>,
    saml2_request_id_cache: Cache<String, ()>,
    auth_response_holding: Cache<String, AuthData>,
    email_token_holding: Cache<String, EmailLoginBody>,
}

async fn get_service_provider()
-> color_eyre::Result<(ServiceProvider, openssl::pkey::PKey<openssl::pkey::Private>)> {
    // let resp = reqwest::get("https://testidpv4.lu.se/idp/shibboleth")
    let resp = reqwest::get("https://mocksaml.com/api/saml/metadata")
        .await?
        .text()
        .await?;
    let idp_metadata: EntityDescriptor = samael::metadata::de::from_str(&resp)?;

    let saml_pk = std::env::var("SAML_PRIVATE_KEY").wrap_err("`SAML_PRIVATE_KEY` not detected")?;
    let saml_pk = base64::prelude::BASE64_STANDARD
        .decode(saml_pk)
        .wrap_err("`SAML_PRIVATE_KEY` not base64 encoded")?;
    let saml_pk = openssl::pkey::PKey::from_rsa(openssl::rsa::Rsa::private_key_from_pem(&saml_pk)?)
        .wrap_err("`SAML_PRIVATE_KEY` not valid base64 encoded PEM private key")?;
    let saml_cert = std::env::var("SAML_CERTIFICATE").wrap_err("`SAML_PUBLIC_KEY` not detected")?;
    let saml_cert = base64::prelude::BASE64_STANDARD
        .decode(saml_cert)
        .wrap_err("`SAML_CERTIFICATE` not base64 encoded")?;
    let saml_cert = openssl::x509::X509::from_pem(&saml_cert)?;
    let saml_cert = saml_cert.to_der()?;
    let saml_cert = samael::crypto::CertificateDer::from(saml_cert);

    let sp = ServiceProviderBuilder::default()
        .entity_id("https://auth.teknologappen.se/saml2/".to_owned())
        .key(saml_pk.clone())
        .certificate(saml_cert)
        .allow_idp_initiated(false)
        .force_authn(true)
        .contact_person(ContactPerson {
            contact_type: Some(ContactType::Technical.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        })
        .idp_metadata(idp_metadata)
        .acs_url("https://auth.teknologappen.se/saml2/acs".to_owned())
        // doesn't actually exist but is required by samael to exist
        .slo_url("https://auth.teknologappen.se/saml2/slo".to_owned())
        .build()?;
    Ok((sp, saml_pk))
}
fn get_jwt_keys() -> color_eyre::Result<(EncodingKey, Vec<u8>)> {
    let key = std::env::var("PRIVATE_KEY").wrap_err("`PRIVATE_KEY` not detected")?;
    let key = base64::prelude::BASE64_STANDARD
        .decode(key)
        .wrap_err("`PRIVATE_KEY` not base64 encoded")?;
    let ed_key = ed25519_dalek::SigningKey::from_pkcs8_der(&key)
        .wrap_err("`PRIVATE_KEY` not valid EdDSA key")?;
    let verifying_key = ed_key.verifying_key();
    let encoding_key = EncodingKey::from_ed_der(&key);
    let public_key = verifying_key.as_bytes().to_vec();
    Ok((encoding_key, public_key))
}
fn get_cookie(refresh_token: Uuid) -> Cookie {
    let mut cookie = Cookie::new_with_str(REFRESH_TOKEN_COOKIE, refresh_token);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::None);
    cookie.set_secure(true);
    cookie.set_max_age(std::time::Duration::from_hours(24 * 365));
    cookie.set_path("/");
    cookie
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let _: Result<PathBuf, dotenvy::Error> = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let (private_key, public_key) = get_jwt_keys()?;

    let db = setup_db(&std::env::var("DATABASE_URL").wrap_err("`DATABASE_URL` not set")?)
        .await
        .wrap_err("Failed to set up the database")
        .suggestion("Start the database with `docker compose up -d`")?;

    let (sp, saml_pk) = get_service_provider().await?;

    let email = if let Ok(url) = std::env::var("SMTP_URL") {
        let user = std::env::var("SMTP_USER").wrap_err("`SMTP_USER` not set")?;
        let password = std::env::var("SMTP_PASSWORD").wrap_err("`SMTP_PASSWORD` not set")?;

        let credentials =
            lettre::transport::smtp::authentication::Credentials::new(user.clone(), password);
        let transport = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::relay(&url)
            .wrap_err("SMTP setup failed")?
            .credentials(credentials)
            .authentication(vec![
                lettre::transport::smtp::authentication::Mechanism::Plain,
            ])
            .build();
        Some((
            user.parse()
                .wrap_err("`SMTP_USER` is not a valid email address")?,
            transport,
        ))
    } else {
        None
    };

    let context = Context {
        db,
        reqwest_client: reqwest::Client::new(),
        email,
        private_key,
        public_key,
        service_provider: sp,
        saml_private_key: saml_pk,
        // keep them for 30 minutes
        saml2_request_id_cache: Cache::builder()
            .time_to_live(std::time::Duration::from_mins(30))
            .build(),
        auth_response_holding: Cache::builder()
            .time_to_live(std::time::Duration::from_mins(30))
            .build(),
        email_token_holding: Cache::builder()
            .time_to_live(std::time::Duration::from_mins(30))
            .build(),
    };
    #[cfg(debug_assertions)]
    let server_url = "http://localhost:8001";
    #[cfg(not(debug_assertions))]
    let server_url = "https://auth.teknologappen.se";
    let api_service = OpenApiService::new(
        MainRouter {
            context: context.clone(),
        },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(server_url);
    let ui = api_service.swagger_ui();
    let saml_service = OpenApiService::new(
        SamlRouter { context },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(server_url);
    let saml_ui = saml_service.swagger_ui();

    let cors = Cors::new()
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        .allow_header("content-type")
        .allow_header("authorization")
        .allow_credentials(true);

    Server::new(TcpListener::bind("[::]:8001"))
        .run(
            Route::new()
                .nest("/", EmbeddedFilesEndpoint::<Website>::new())
                .nest("/api/v0", api_service)
                .nest("/api/v0/docs", ui)
                .nest("/saml2", saml_service)
                .nest("/saml2/docs", saml_ui)
                .with(cors)
                .with(CookieJarManager::new()),
        )
        .await?;

    Ok(())
}

async fn setup_db(db_url: &str) -> color_eyre::Result<PgPool> {
    if !Postgres::database_exists(db_url)
        .await
        .wrap_err("Failed to check if database exists")?
    {
        Postgres::create_database(db_url).await?;
    }

    let db = PgPoolOptions::new()
        .max_connections(50)
        .connect(db_url)
        .await
        .wrap_err("Failed to create database pool")?;

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .wrap_err("Failed to run migrations")?;

    Ok(db)
}

#[derive(Object)]
struct RefreshResponse {
    access_token: String,
}
#[derive(ApiResponse)]
enum RefreshError {
    /// No origin, the request must be CORS.
    #[oai(status = 400)]
    NoOrigin,
    /// Returns when the user either doesn't have a token or the token is invalid.
    #[oai(status = 401)]
    TokenInvalid,
    /// Unknown internal error.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Serialize)]
struct Claims {
    sub: String,
}
#[derive(Serialize, Clone, Debug)]
struct UserData {
    /// User ID.
    sub: String,
    full_name: String,
    email: String,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
pub enum ConfirmResponseError {
    /// Confirmation took too long.
    #[oai(status = 400)]
    CacheFlushed,
    /// Authentication is not valid!
    #[oai(status = 400)]
    AuthNotValid,
    /// Unknown internal error.
    #[oai(status = 500)]
    Unknown,
    /// Database error.
    #[oai(status = 500)]
    DbError,
    /// Callback post request failed. See server logs.
    #[oai(status = 503)]
    CallbackFailed,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
pub enum RedirectResponseError {
    /// The client must send origin, i.e. this must be a CORS request. It must also be UTF-8.
    #[oai(status = 400)]
    InvalidOrigin,
    /// Unknown internal server error in URL creation.
    /// See logs.
    #[oai(status = 500)]
    Unknown,
}
#[derive(Object)]
pub struct ConfirmRequest {
    accepted: bool,
    id: String,
}
#[derive(Object)]
pub struct RedirectBody {
    continue_url: String,
    callback_url: Option<String>,
}
#[derive(Object, Clone)]
struct EmailLoginBody {
    email: String,
    name: String,
    id: String,
}
#[derive(Object)]
struct EmailApproveBody {
    token: String,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum EmailLoginResponseError {
    /// The client took too long.
    #[oai(status = 401)]
    Timeout,
    /// Invalid e-mail address.
    #[oai(status = 400)]
    InvalidEmail,
    /// Invalid name.
    #[oai(status = 400)]
    InvalidName,
    /// Error sending email.
    #[oai(status = 500)]
    EmailError,
}
#[derive(Object, Clone)]
struct TestLoginBody {
    stil_id: String,
    name: String,
    id: String,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum TestLoginResponseError {
    /// No such login ID.
    #[oai(status = 401)]
    InvalidId,
}

#[derive(Clone)]
struct MainRouter {
    context: Context,
}
impl Deref for MainRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
#[OpenApi]
impl MainRouter {
    /// Returns the key as DER.
    #[oai(path = "/verify-key.der", method = "get")]
    async fn get_verify_key(&self) -> Response<Binary<Vec<u8>>> {
        Response::new(Binary(self.public_key.clone()))
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/refresh", method = "post")]
    async fn refresh(
        &self,
        headers: &poem::http::HeaderMap,
        cookies: &CookieJar,
    ) -> Result<Json<RefreshResponse>, RefreshError> {
        let origin = headers
            .get("origin")
            .and_then(|header| header.to_str().ok())
            .ok_or(RefreshError::NoOrigin)?;

        let Some(refresh_token) = cookies.get(REFRESH_TOKEN_COOKIE) else {
            return Err(RefreshError::TokenInvalid);
        };
        let Ok(refresh_token) = refresh_token.value_str().parse::<Uuid>() else {
            return Err(RefreshError::TokenInvalid);
        };

        let mut conn = self
            .db
            .begin()
            .await
            .inspect_err(|err| {
                error!("failed to open DB transaction: {err}");
            })
            .map_err(|_| RefreshError::Unknown)?;
        let get_query = sqlx::query!(
            "select * from auth_refresh_tokens where refresh_token = $1 and domain = $2",
            refresh_token,
            origin,
        )
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| RefreshError::TokenInvalid)?;

        sqlx::query!(
            "delete from auth_refresh_tokens where refresh_token = $1 and domain = $2",
            refresh_token,
            origin,
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::Unknown)?;

        let new_refresh = Uuid::new_v4();
        sqlx::query!(
            "insert into auth_refresh_tokens values ($1, $2, $3)",
            get_query.user_id,
            get_query.domain,
            new_refresh
        )
        .execute(&mut *conn)
        .await
        .map_err(|_| RefreshError::Unknown)?;

        conn.commit()
            .await
            .inspect_err(|err| {
                error!("failed to commit DB transaction: {err}");
            })
            .map_err(|_| RefreshError::Unknown)?;

        let claims = Claims {
            sub: get_query.user_id,
        };
        let access_token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA),
            &JwtData::new(claims),
            &self.private_key,
        )
        .inspect_err(|err| {
            error!("failed to encode JWT: {err}");
        })
        .map_err(|_| RefreshError::Unknown)?;

        cookies.add(get_cookie(new_refresh));

        Ok(Json(RefreshResponse { access_token }))
    }
    /// Removes the refresh token.
    #[oai(path = "/logout", method = "post")]
    async fn logout(&self, cookies: &CookieJar) {
        cookies.remove(REFRESH_TOKEN_COOKIE);
    }
    /// Verifies that your access token is correct.
    #[oai(path = "/verify-access-token", method = "post")]
    async fn verify_access_token(&self, _user: User) {}
    #[oai(path = "/confirm-datasharing", method = "post")]
    async fn confirm_datasharing(
        &self,
        body: Json<ConfirmRequest>,
        cookies: &CookieJar,
    ) -> Result<PlainText<String>, ConfirmResponseError> {
        let Some(data) = self.auth_response_holding.get(&body.id) else {
            warn!(
                "Tried to confirm datasharing with an ID which is not in the database ({})",
                body.id
            );
            return Err(ConfirmResponseError::CacheFlushed);
        };
        if !ALLOWED_DOMAINS.contains(&data.origin.as_str()) {
            return Ok(PlainText(format!(
                "{}{}validated=false",
                data.continue_url,
                if data.continue_url.contains('?') {
                    '&'
                } else {
                    '?'
                },
            )));
        }
        let Some(user_data) = data.validated_user else {
            warn!(
                "Tried to confirm datasharing for a request which was not validated ({})",
                body.id
            );
            return Err(ConfirmResponseError::AuthNotValid);
        };

        if body.accepted {
            if let Some(cb_url) = &data.callback_url {
                let token = jsonwebtoken::encode(
                    &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA),
                    &JwtData::new(&user_data),
                    &self.private_key,
                )
                .inspect_err(|err| error!("could not create callback token: {err}"))
                .map_err(|_| ConfirmResponseError::Unknown)?;
                self.reqwest_client
                    .post(cb_url)
                    .body(token)
                    .send()
                    .await
                    .inspect_err(|err| error!("auth callback POST failed: {err}"))
                    .map_err(|_| ConfirmResponseError::CallbackFailed)?;
            }

            let refresh_token = Uuid::new_v4();
            sqlx::query!(
                "insert into auth_refresh_tokens values ($1, $2, $3)",
                user_data.sub,
                data.origin,
                refresh_token
            )
            .execute(&self.db)
            .await
            .inspect_err(|err| error!("Error inserting refresh token into DB: {err}"))
            .map_err(|_| ConfirmResponseError::DbError)?;
            cookies.add(get_cookie(refresh_token));
        }

        self.auth_response_holding.invalidate(&body.id);
        Ok(PlainText(format!(
            "{}{}validated={}",
            data.continue_url,
            if data.continue_url.contains('?') {
                '&'
            } else {
                '?'
            },
            body.accepted
        )))
    }

    #[allow(clippy::unused_self, reason = "makes the developer experience nicer")]
    fn check_redirect_provider<'a>(
        &self,
        headers: &'a poem::http::HeaderMap,
        body: &Json<RedirectBody>,
    ) -> Result<&'a str, RedirectResponseError> {
        let origin = headers
            .get("origin")
            .and_then(|header| header.to_str().ok())
            .ok_or(RedirectResponseError::InvalidOrigin)?;
        if let Some(cb_url) = &body.callback_url {
            let cb_url: poem::http::Uri = cb_url
                .parse()
                .map_err(|_| RedirectResponseError::InvalidOrigin)?;
            if Some(origin) != cb_url.host() {
                return Err(RedirectResponseError::InvalidOrigin);
            }
        }
        Ok(origin)
    }
    fn redirect_provider(
        &self,
        body: &Json<RedirectBody>,
        id: &str,
        origin: &str,
        url: String,
    ) -> PlainText<String> {
        let data = AuthData {
            origin: origin.to_owned(),
            callback_url: body.callback_url.clone(),
            continue_url: body.continue_url.clone(),

            validated_user: None,
        };
        self.auth_response_holding.insert(id.to_owned(), data);

        PlainText(url)
    }
    /// Get URL to redirect user to to authenticate by LU SSO
    #[oai(path = "/providers/lu", method = "post")]
    async fn lu(
        &self,
        headers: &poem::http::HeaderMap,
        body: Json<RedirectBody>,
    ) -> Result<PlainText<String>, RedirectResponseError> {
        let origin = self.check_redirect_provider(headers, &body)?;

        let req = self
            .service_provider
            // .make_authentication_request("https://testidpv4.lu.se/idp/profile/SAML2/Redirect/SSO")
            .make_authentication_request("https://mocksaml.com/api/saml/sso")
            .inspect_err(|err| error!("Failed to make LU SSO request {err}"))
            .map_err(|_| RedirectResponseError::Unknown)?;
        let redirect = req
            .signed_redirect("", &self.saml_private_key)
            .inspect_err(|err| error!("Failed to make LU SSO redirect {err}"))
            .map_err(|_| RedirectResponseError::Unknown)?
            .ok_or_else(|| {
                error!("Failed to create LU SSO link");
                RedirectResponseError::Unknown
            })?;
        self.saml2_request_id_cache.insert(req.id.clone(), ());
        debug!("Added ID {} to auth request id cache", req.id);

        Ok(self.redirect_provider(&body, &req.id, origin, redirect.to_string()))
    }
    #[oai(path = "/providers/email", method = "post")]
    async fn email(
        &self,
        headers: &poem::http::HeaderMap,
        body: Json<RedirectBody>,
    ) -> Result<PlainText<String>, RedirectResponseError> {
        let origin = self.check_redirect_provider(headers, &body)?;
        let id = random_id();
        let redirect = format!("https://auth.teknologappen.se/providers/email/?id={id}");

        Ok(self.redirect_provider(&body, &id, origin, redirect))
    }
    #[oai(path = "/providers/test", method = "post")]
    async fn test_provider(
        &self,
        headers: &poem::http::HeaderMap,
        body: Json<RedirectBody>,
    ) -> Result<PlainText<String>, RedirectResponseError> {
        let origin = self.check_redirect_provider(headers, &body)?;
        let id = random_id();
        let redirect = format!("https://auth.teknologappen.se/providers/test/?id={id}");

        Ok(self.redirect_provider(&body, &id, origin, redirect))
    }
    /// Corresponds to the login happening at the `IdP` in `SAML2`.
    #[oai(path = "/providers/email/login", method = "post")]
    async fn email_login(&self, body: Json<EmailLoginBody>) -> Result<(), EmailLoginResponseError> {
        if !self.auth_response_holding.contains_key(&body.id) {
            return Err(EmailLoginResponseError::Timeout);
        }
        if !body.name.contains(' ') || body.name.len() < 5 {
            return Err(EmailLoginResponseError::InvalidName);
        }
        let token = random_id();
        // having this as format_args made the await point for lettre fail because format_args is
        // not Send??
        let link = format!("https://auth.teknologappen.se/providers/email/approve/?token={token}");
        if let Some((from, email)) = &self.email {
            let html = format!(
                "<p>Någon har försökt logga in med denna e-post adress. Om detta inte var du bör du slänga detta mailet. Tryck på länken för att logga in.</p><p><a href='{link}'>{link}</a>"
            );
            let message = lettre::Message::builder()
                .from(lettre::message::Mailbox::new(
                    Some("Teknologappens inloggningstjänst".to_owned()),
                    from.clone(),
                ))
                .to(body
                    .email
                    .parse::<lettre::Address>()
                    .map_err(|_| EmailLoginResponseError::InvalidEmail)?
                    .into())
                .subject("Logga in med teknologappens inloggningstjänst")
                .header(lettre::message::header::ContentType::TEXT_HTML)
                .body(html)
                .inspect_err(|err| error!("Error when formatting a mail: {err}"))
                .map_err(|_| EmailLoginResponseError::EmailError)?;
            email
                .send(message)
                .await
                .inspect_err(|err| error!("failed to send email: {err}"))
                .map_err(|_| EmailLoginResponseError::EmailError)?;
        } else {
            println!(
                "Someone tried to log in with the email {}. Click the link below to continue.",
                body.email
            );
            println!("{link}");
        }

        self.email_token_holding.insert(token, (*body).clone());

        Ok(())
    }
    /// Corresponds to acs in saml
    #[oai(path = "/providers/email/approve", method = "post")]
    async fn mail_approve(
        &self,
        body: Json<EmailApproveBody>,
    ) -> Result<PlainText<String>, EmailLoginResponseError> {
        let Some(login_data) = self.email_token_holding.get(&body.token) else {
            return Err(EmailLoginResponseError::Timeout);
        };
        let Some(mut data) = self.auth_response_holding.get(&login_data.id) else {
            return Err(EmailLoginResponseError::Timeout);
        };
        data.validated_user = Some(UserData {
            sub: format!("mail:{}", login_data.email),
            full_name: login_data.name,
            email: login_data.email,
        });
        self.auth_response_holding
            .insert(login_data.id.clone(), data.clone());

        Ok(PlainText(format!(
            "/confirm-datasharing/?id={}&origin={}",
            login_data.id, data.origin
        )))
    }
    /// Corresponds to acs in saml
    #[oai(path = "/providers/test/approve", method = "post")]
    async fn test_approve(
        &self,
        body: Json<TestLoginBody>,
    ) -> Result<PlainText<String>, TestLoginResponseError> {
        let Some(mut data) = self.auth_response_holding.get(&body.id) else {
            return Err(TestLoginResponseError::InvalidId);
        };
        data.validated_user = Some(UserData {
            sub: format!("test:{}", body.stil_id),
            full_name: body.name.clone(),
            email: format!("{}@student.lu.se", body.stil_id),
        });
        self.auth_response_holding
            .insert(body.id.clone(), data.clone());

        Ok(PlainText(format!(
            "/confirm-datasharing/?id={}&origin={}",
            body.id, data.origin
        )))
    }
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum MetadataResponseError {
    /// Unable to produce correct metadata, invalid configuration.
    #[oai(status = 500)]
    MetadataInvalid,
}
#[derive(ApiResponse, Debug, Clone, Copy)]
enum AcsResponseError {
    /// No `SAMLResponse` prop.
    #[oai(status = 400)]
    NoSamlResponse,
    /// Invalid ACS response.
    #[oai(status = 400)]
    InvalidAcsResponse,
    /// SAML response took too long.
    #[oai(status = 400)]
    CacheFlushed,
}
#[derive(Clone)]
struct SamlRouter {
    context: Context,
}
impl Deref for SamlRouter {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
fn add_metadata(metadata: &mut EntityDescriptor) -> Result<(), MetadataResponseError> {
    let org_name = vec![
        LocalizedName {
            lang: Some("se".into()),
            value: "Utvecklarna bakom Teknologappen".into(),
        },
        LocalizedName {
            lang: Some("en".into()),
            value: "The developers behind Teknologappen".into(),
        },
    ];
    metadata.organization = Some(samael::metadata::Organization {
        organization_names: Some(org_name.clone()),
        organization_display_names: Some(org_name),
        organization_urls: Some(vec![
            LocalizedUri {
                lang: Some("se".into()),
                value: "https://teknologappen.se".into(),
            },
            LocalizedUri {
                lang: Some("en".into()),
                value: "https://teknologappen.se".into(),
            },
        ]),
    });
    let Some(sp_desc) = metadata
        .sp_sso_descriptors
        .as_mut()
        .and_then(|descs| descs.first_mut())
    else {
        error!("Failed to get sp sso descriptor");
        return Err(MetadataResponseError::MetadataInvalid);
    };
    metadata.contact_person = Some(vec![
        ContactPerson {
            contact_type: Some(ContactType::Technical.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        },
        ContactPerson {
            contact_type: Some(ContactType::Support.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        },
        ContactPerson {
            contact_type: Some(ContactType::Administrative.value().to_owned()),
            company: Some("E-sektionen inom TLTH".to_owned()),
            given_name: Some("Informationschef".to_owned()),
            sur_name: None,
            email_addresses: Some(vec!["informationschef@esek.se".to_owned()]),
            telephone_numbers: None,
        },
    ]);
    let attribute_name_format = "urn:oasis:names:tc:SAML:2.0:attrname-format:uri";
    sp_desc.attribute_consuming_services = Some(vec![AttributeConsumingService {
        index: 1,
        is_default: None,
        service_names: vec![
            LocalizedName {
                lang: Some("sv".into()),
                value: "Teknologappens inloggningstjänst".into(),
            },
            LocalizedName {
                lang: Some("en".into()),
                value: "The login service for teknologappen".into(),
            },
        ],
        service_descriptions: None,
        request_attributes: vec![
            RequestedAttribute {
                friendly_name: Some("samlSubjectID".into()),
                name: "urn:oasis:names:tc:SAML:attribute:subject-id".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
            RequestedAttribute {
                friendly_name: Some("mail".into()),
                name: "urn:oid:0.9.2342.19200300.100.1.3".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
            RequestedAttribute {
                friendly_name: Some("displayName".into()),
                name: "urn:oid:2.16.840.1.113730.3.1.241".into(),
                name_format: Some(attribute_name_format.into()),
                values: None,
                is_required: Some(true),
            },
        ],
    }]);
    // sp_desc.name_id_formats = Some(vec!["urn:oasis:names:tc:SAML:2.0:nameid-format:transient".into()]);
    Ok(())
}
fn add_metadata_extensions(meta: &mut xmltree::Element) -> Result<usize, MetadataResponseError> {
    // the xmlns are needed for parsing, they are removed later. Copied from an example SP
    // metadata: https://metadata.qa.swamid.se/?rawXML=1361
    let security_contact_person = r#"<md:ContactPerson contactType="other" remd:contactType="http://refeds.org/metadata/contactType/security" xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:remd="http://refeds.org/metadata">
    <md:Company>E-sektionen inom TLTH</md:Company>
    <md:GivenName>Informationschef</md:GivenName>
    <md:EmailAddress>informationschef@esek.se</md:EmailAddress>
</md:ContactPerson>"#;
    let descriptor_extensions = r#"<md:Extensions xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:mdattr="urn:oasis:names:tc:SAML:metadata:attribute" xmlns:samla="urn:oasis:names:tc:SAML:2.0:assertion" xmlns:mdrpi="urn:oasis:names:tc:SAML:metadata:rpi" xmlns:mdui="urn:oasis:names:tc:SAML:metadata:ui" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:remd="http://refeds.org/metadata">
    <mdattr:EntityAttributes>
        <samla:Attribute Name="http://macedir.org/entity-category" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:uri">
            <samla:AttributeValue>https://refeds.org/category/personalized</samla:AttributeValue>
        </samla:Attribute>
    </mdattr:EntityAttributes>
        <mdrpi:RegistrationInfo registrationAuthority="http://www.swamid.se/" registrationInstant="2026-05-13T11:22:11Z">
        <mdrpi:RegistrationPolicy xml:lang="en">http://swamid.se/policy/mdrps</mdrpi:RegistrationPolicy>
    </mdrpi:RegistrationInfo>
</md:Extensions>"#;
    let spsso_extensions = r#"<md:Extensions xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:mdattr="urn:oasis:names:tc:SAML:metadata:attribute" xmlns:samla="urn:oasis:names:tc:SAML:2.0:assertion" xmlns:mdrpi="urn:oasis:names:tc:SAML:metadata:rpi" xmlns:mdui="urn:oasis:names:tc:SAML:metadata:ui" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:remd="http://refeds.org/metadata">
    <mdui:UIInfo>
        <mdui:DisplayName xml:lang="en">Teknologappen and guild logins for members of TLTH</mdui:DisplayName>
        <mdui:DisplayName xml:lang="sv">Teknologappen och sektionsinlogg för medlemmar i TLTH</mdui:DisplayName>
        <mdui:Description xml:lang="sv">Teknologappenlogin, utvecklat av E, D, och F-sektionen</mdui:Description>
        <mdui:Description xml:lang="en">Teknologappen login, developed by the E, D, and F guilds</mdui:Description>
        <mdui:InformationURL xml:lang="sv">https://teknologappen.se</mdui:InformationURL>
        <mdui:InformationURL xml:lang="en">https://teknologappen.se</mdui:InformationURL>
        <mdui:PrivacyStatementURL xml:lang="en">https://auth.teknologappen.se/privacy-statement/</mdui:PrivacyStatementURL>
        <mdui:PrivacyStatementURL xml:lang="sv">https://auth.teknologappen.se/privacy-statement/</mdui:PrivacyStatementURL>
    </mdui:UIInfo>
</md:Extensions>"#;
    let mut sec_meta = xmltree::Element::parse(Cursor::new(security_contact_person))
        .inspect_err(|err| error!("Failed to parse metadata: {err}"))
        .map_err(|_| MetadataResponseError::MetadataInvalid)?;
    let mut desc_meta = xmltree::Element::parse(Cursor::new(descriptor_extensions))
        .inspect_err(|err| error!("Failed to parse metadata: {err}"))
        .map_err(|_| MetadataResponseError::MetadataInvalid)?;
    let mut spsso_meta = xmltree::Element::parse(Cursor::new(spsso_extensions))
        .inspect_err(|err| error!("Failed to parse metadata: {err}"))
        .map_err(|_| MetadataResponseError::MetadataInvalid)?;
    meta.namespaces = desc_meta.namespaces.take();
    sec_meta.namespaces = None;
    spsso_meta.namespaces = None;

    meta.children.push(XMLNode::Element(sec_meta));
    meta.children.push(XMLNode::Element(desc_meta));

    let Some(spsso) = meta.get_mut_child("SPSSODescriptor") else {
        error!("Metadata is not an object!");
        return Err(MetadataResponseError::MetadataInvalid);
    };
    spsso.children.push(XMLNode::Element(spsso_meta));

    Ok(descriptor_extensions.len() + spsso_extensions.len())
}
#[OpenApi]
impl SamlRouter {
    /// Returns the SAML2 metadata.
    ///
    /// The body actually is `application/xml` but since [`poem-openapi`] is cringe I can't just
    /// add a string as an XML response.
    #[oai(path = "/metadata", method = "get")]
    async fn metadata(&self) -> Result<Response<Binary<Vec<u8>>>, MetadataResponseError> {
        let mut metadata = self
            .service_provider
            .metadata()
            .inspect_err(|err| error!("Failed to get metadata: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;
        add_metadata(&mut metadata)?;

        let metadata = metadata
            .to_string()
            .inspect_err(|err| error!("Failed to convert metadata to string: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;

        let mut meta = xmltree::Element::parse(Cursor::new(&metadata))
            .inspect_err(|err| error!("Failed to parse metadata: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;

        let exts_len = add_metadata_extensions(&mut meta)?;

        let mut metadata = Cursor::new(Vec::with_capacity(metadata.len() + exts_len + 100));
        meta.write(&mut metadata)
            .inspect_err(|err| error!("Failed to serialize updated metadata: {err}"))
            .map_err(|_| MetadataResponseError::MetadataInvalid)?;
        let metadata = metadata.into_inner();
        Ok(
            Response::new(Binary(metadata))
                .header("content-type", "application/xml; charset=utf-8"),
        )
    }
    /// Get JWT access token and a new refresh token.
    #[oai(path = "/acs", method = "post")]
    #[allow(clippy::panic, reason = "yes")]
    async fn acs(
        &self,
        body: Form<HashMap<String, String>>,
    ) -> Result<Response<()>, AcsResponseError> {
        // we'd want the library to take an iterator instead of &[&str]
        let ids: Vec<_> = self
            .saml2_request_id_cache
            .iter()
            .map(|entry| entry.key().to_owned())
            .collect();
        let ids: Vec<_> = ids.iter().map(String::as_str).collect();

        let saml_response = body
            .get("SAMLResponse")
            .ok_or(AcsResponseError::NoSamlResponse)?;
        let ass = self
            .service_provider
            .parse_base64_response(saml_response, Some(&ids))
            .inspect_err(|err| error!("Invalid ACS response: {err}"))
            .map_err(|_| AcsResponseError::InvalidAcsResponse)?;
        let Some(request_id) = ass
            .subject
            .as_ref()
            .and_then(|sub| sub.subject_confirmations.as_ref())
            .and_then(|confs| confs.first())
            .and_then(|conf| conf.subject_confirmation_data.as_ref())
            .and_then(|conf_data| conf_data.in_response_to.as_ref())
        else {
            return Err(AcsResponseError::InvalidAcsResponse);
        };
        let Some(mut data) = self.auth_response_holding.get(request_id) else {
            return Err(AcsResponseError::CacheFlushed);
        };
        println!("{ass:#?}");
        data.validated_user = ass
            .subject
            .as_ref()
            .and_then(|sub| sub.name_id.as_ref())
            .map(|name_id| UserData {
                sub: format!("lund-university:{}", name_id.value.clone()),
                email: String::new(),
                full_name: "Erika Davidssona".to_owned(),
            });
        self.auth_response_holding
            .insert(request_id.clone(), data.clone());
        Ok(Response::new(()).status(StatusCode::SEE_OTHER).header(
            "location",
            format!(
                "/confirm-datasharing/?id={}&origin={}",
                request_id, data.origin
            ),
        ))
    }
}
