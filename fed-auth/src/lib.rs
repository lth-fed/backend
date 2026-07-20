#![allow(clippy::unused_async, reason = "OpenAPI requires async handlers")]
#![allow(
    missing_debug_implementations,
    reason = "we can't add debug to e.g. Context"
)]
#![allow(clippy::same_name_method, reason = "rust_embed uses it")]
use bin_common::get_otel;
use fed_auth_verifier::{AuthUrl, JwkContext};
use opentelemetry::trace::TracerProvider as _;
use poem::EndpointExt as _;
use poem::endpoint::EmbeddedFilesEndpoint;
use poem::http::Method;

use poem::middleware::{Cors, OpenTelemetryMetrics, OpenTelemetryTracing, SetHeader};

use poem::Route;
use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi, OpenApiService};
use reqwest::header::CACHE_CONTROL;

mod api;
mod context;
mod jwt;
mod oidc;
mod saml2;

pub use bin_common::PgPool;
pub(crate) use context::Context;

#[cfg(debug_assertions)]
pub(crate) const API_DOMAIN: &str = "http://localhost:8001";
#[cfg(not(debug_assertions))]
pub(crate) const API_DOMAIN: &str = "https://auth.teknologappen.se";
pub(crate) const WEBSITE_DOMAIN: &str = API_DOMAIN;

#[derive(rust_embed::Embed)]
#[cfg_attr(
    not(all(
        debug_assertions,
        feature = "ci-test-dont-use-when-building-for-production"
    )),
    folder = "../../frontend/auth/build"
)]
#[cfg_attr(
    all(
        debug_assertions,
        feature = "ci-test-dont-use-when-building-for-production"
    ),
    folder = "."
)]
struct Website;

fn random_id() -> String {
    // hex-formatted random string
    format!("{:X}", rand::random::<u128>())
}

/// # Errors
///
/// If the endpoint fails to set up, often because env variables / database is missing.
pub async fn get_endpoint(test_db: Option<PgPool>) -> color_eyre::Result<impl poem::Endpoint> {
    let tracer = get_otel(env!("CARGO_PKG_NAME"), test_db.is_some())?;

    let context = Context::new(test_db).await?;
    let auth_ctx = JwkContext::<AuthUrl>::from_jwks("", context.jwks.clone());
    let server_url = API_DOMAIN;
    let api_service = OpenApiService::new(
        api::MainRouter {
            context: context.clone(),
        },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(format!("{server_url}/api/v0"));
    let ui = api_service.swagger_ui();
    let spec = api_service.spec_endpoint();
    let oidc_service = OpenApiService::new(
        oidc::MainRouter {
            context: context.clone(),
        },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(format!("{server_url}/oidc/v1"));
    let oidc_ui = oidc_service.swagger_ui();
    let oidc_spec = oidc_service.spec_endpoint();
    let saml_service = OpenApiService::new(
        saml2::SamlRouter { context },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(format!("{server_url}/saml2/"));
    let saml_ui = saml_service.swagger_ui();
    let saml_spec = saml_service.spec_endpoint();

    let well_known = OpenApiService::new(
        WellKnownRouter,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(format!("{server_url}/.well-known/"));

    let cors = Cors::new()
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        .allow_header("content-type")
        .allow_header("authorization")
        .allow_credentials(true);

    let route = Route::new()
        .nest("/", EmbeddedFilesEndpoint::<Website>::new())
        .nest(
            "/api/v0",
            api_service.with(SetHeader::new().overriding(CACHE_CONTROL, "no-cache")),
        )
        .nest("/api/v0/docs", ui)
        .nest("/api/v0/spec.json", spec)
        .nest(
            "/oidc/v1",
            oidc_service
                .data(auth_ctx)
                .with(SetHeader::new().overriding(CACHE_CONTROL, "no-cache")),
        )
        .nest("/oidc/v1/docs", oidc_ui)
        .nest("/oidc/v1/spec.json", oidc_spec)
        .nest("/saml2", saml_service)
        .nest("/saml2/docs", saml_ui)
        .nest("/saml2/spec.json", saml_spec)
        .nest("/.well-known", well_known)
        .with(OpenTelemetryMetrics::new())
        .with(OpenTelemetryTracing::new(
            tracer.tracer(env!("CARGO_PKG_NAME")),
        ))
        .with(cors);
    Ok(route)
}

#[derive(Object)]
struct WellKnownOidc {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
    jwks_uri: String,
    response_types_supported: Vec<String>,
    id_token_signing_alg_values_supported: Vec<String>,
    scopes_supported: Vec<String>,
    token_endpoint_auth_methods_supported: Vec<String>,
    code_challenge_methods_supported: Vec<String>,
    grant_types_supported: Vec<String>,
}
impl WellKnownOidc {
    fn new() -> Self {
        Self {
            issuer: API_DOMAIN.to_owned(),
            authorization_endpoint: format!("{API_DOMAIN}/oidc/v1/authorize"),
            token_endpoint: format!("{API_DOMAIN}/oidc/v1/token"),
            userinfo_endpoint: format!("{API_DOMAIN}/oidc/v1/userinfo"),
            jwks_uri: format!("{API_DOMAIN}/oidc/v1/certs"),
            response_types_supported: vec!["code".to_owned()],
            id_token_signing_alg_values_supported: vec!["EdDSA".to_owned()],
            scopes_supported: vec!["openid".to_owned()],
            token_endpoint_auth_methods_supported: vec!["none".to_owned()],
            code_challenge_methods_supported: vec!["S256".to_owned()],
            grant_types_supported: vec![
                "authorization_code".to_owned(),
                "refresh_token".to_owned(),
            ],
        }
    }
}
#[derive(Clone)]
pub(crate) struct WellKnownRouter;
#[OpenApi]
impl WellKnownRouter {
    #[oai(path = "/openid-configuration", method = "get")]
    async fn oidc(&self) -> Json<WellKnownOidc> {
        Json(WellKnownOidc::new())
    }
}
