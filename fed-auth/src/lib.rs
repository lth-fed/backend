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
use poem::EndpointExt as _;
use poem::endpoint::EmbeddedFilesEndpoint;
use poem::http::Method;
use poem::middleware::{CookieJarManager, Cors};

use poem::Route;
use poem_openapi::OpenApiService;
use sqlx::PgPool;

mod api;
mod context;
mod cookie;
mod jwt;
mod saml2;

pub(crate) use context::Context;

#[derive(rust_embed::Embed)]
#[folder = "../../frontend/auth/build"]
struct Website;

fn random_id() -> String {
    // hex-formatted random string
    format!("{:X}", rand::random::<u128>())
}

/// # Errors
///
/// If the endpoint fails to set up, often because env variables / database is missing.
pub async fn get_endpoint(db: Option<PgPool>) -> color_eyre::Result<impl poem::Endpoint> {
    let context = Context::new(db).await?;
    #[cfg(debug_assertions)]
    let server_url = "http://localhost:8001";
    #[cfg(not(debug_assertions))]
    let server_url = "https://auth.teknologappen.se";
    let api_service = OpenApiService::new(
        api::MainRouter {
            context: context.clone(),
        },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(server_url);
    let ui = api_service.swagger_ui();
    let saml_service = OpenApiService::new(
        saml2::SamlRouter { context },
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

    let route = Route::new()
        .nest("/", EmbeddedFilesEndpoint::<Website>::new())
        .nest("/api/v0", api_service)
        .nest("/api/v0/docs", ui)
        .nest("/saml2", saml_service)
        .nest("/saml2/docs", saml_ui)
        .with(cors)
        .with(CookieJarManager::new());
    Ok(route)
}
