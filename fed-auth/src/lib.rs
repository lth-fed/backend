#![allow(clippy::unused_async, reason = "OpenAPI requires async handlers")]
#![allow(
    missing_debug_implementations,
    reason = "we can't add debug to e.g. Context"
)]
#![allow(clippy::same_name_method, reason = "rust_embed uses it")]
use bin_common::get_otel;
use fed_auth_verifier::AuthContext;
use opentelemetry::trace::TracerProvider;
use poem::EndpointExt as _;
use poem::endpoint::EmbeddedFilesEndpoint;
use poem::http::Method;
use poem::middleware::{CookieJarManager, Cors, OpenTelemetryMetrics, OpenTelemetryTracing};

use poem::Route;
use poem_openapi::OpenApiService;

mod api;
mod context;
mod cookie;
mod jwt;
mod saml2;

pub use bin_common::PgPool;
pub(crate) use context::Context;

#[cfg(debug_assertions)]
pub(crate) const DOMAIN: &str = "http://localhost:8001";
#[cfg(not(debug_assertions))]
pub(crate) const DOMAIN: &str = "https://auth.teknologappen.se";

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
pub async fn get_endpoint(db: Option<PgPool>) -> color_eyre::Result<impl poem::Endpoint> {
    let tracer = get_otel(env!("CARGO_PKG_NAME"))?;

    let context = Context::new(db).await?;
    let auth_ctx =
        AuthContext::from_decoding_key(jsonwebtoken::DecodingKey::from_ed_der(&context.public_key));
    let server_url = DOMAIN;
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
    let spec = api_service.spec_endpoint();
    let saml_service = OpenApiService::new(
        saml2::SamlRouter { context },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server(server_url);
    let saml_ui = saml_service.swagger_ui();
    let saml_spec = saml_service.spec_endpoint();

    let cors = Cors::new()
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        .allow_header("content-type")
        .allow_header("authorization")
        .allow_credentials(true);

    let route = Route::new()
        .nest("/", EmbeddedFilesEndpoint::<Website>::new())
        .nest("/api/v0", api_service.data(auth_ctx))
        .nest("/api/v0/docs", ui)
        .nest("/api/v0/spec.json", spec)
        .nest("/saml2", saml_service)
        .nest("/saml2/docs", saml_ui)
        .nest("/saml2/spec.json", saml_spec)
        .with(OpenTelemetryMetrics::new())
        .with(OpenTelemetryTracing::new(
            tracer.tracer(env!("CARGO_PKG_NAME")),
        ))
        .with(cors)
        .with(CookieJarManager::new());
    Ok(route)
}
