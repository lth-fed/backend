use std::collections::HashMap;

use bin_common::get_otel;
use fed_auth_verifier::AuthContext;
use opentelemetry::trace::TracerProvider as _;
use poem::http::Method;
use poem::middleware::{Cors, OpenTelemetryMetrics, OpenTelemetryTracing};
use poem::{Endpoint, EndpointExt as _, Route};
use poem_openapi::OpenApiService;

pub mod activities;
pub mod context;
pub mod group;
pub mod healthcheck;
pub mod ticket;
pub mod user;

pub use bin_common::PgPool;
pub use context::Context;
pub use minilith_errors::*;

pub type DbInternationalizedString = sqlx::types::Json<InternationalizedString>;
// eventually implement Deserialize ourselves with restrictions
#[derive(Debug, Clone, poem_openapi::NewType, serde::Serialize, serde::Deserialize)]
#[oai(from_multipart = false, from_parameter = false, to_header = false)]
#[serde(transparent)]
pub struct InternationalizedString(HashMap<String, String>);
impl InternationalizedString {
    /// # Panics
    ///
    /// None.
    #[must_use]
    pub fn to_json_value(self) -> serde_json::Value {
        #[allow(clippy::expect_used, reason = "See string below")]
        serde_json::to_value(self.0)
            .expect("we know a hashmap will always serialize & we also know it has string keys")
    }
}
impl From<DbInternationalizedString> for InternationalizedString {
    fn from(value: DbInternationalizedString) -> Self {
        value.0
    }
}

/// # Errors
///
/// See [`Context::new`].
pub async fn get_endpoint(test_db: Option<PgPool>) -> color_eyre::Result<impl Endpoint> {
    let otel = get_otel(env!("CARGO_PKG_NAME"), test_db.is_some())?;

    let context = Context::new(test_db, true).await?;
    let auth_context = AuthContext::new().await?;
    let api_service = OpenApiService::new(
        (
            activities::Router {
                context: context.clone(),
            },
            group::Router {
                context: context.clone(),
            },
            ticket::Router {
                context: context.clone(),
            },
            healthcheck::Router {
                context: context.clone(),
            },
            user::Router {
                context: context.clone(),
            },
        ),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server("http://localhost:8000/v0");
    let ui = api_service.swagger_ui();
    let spec = api_service.spec_endpoint();

    let cors = Cors::new()
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        .allow_header("content-type")
        .allow_header("authorization")
        .allow_credentials(true);

    Ok(Route::new()
        .nest("/v0", api_service.data(auth_context))
        .nest("/v0/docs", ui)
        .nest("/v0/spec.json", spec)
        .with(OpenTelemetryMetrics::new())
        .with(OpenTelemetryTracing::new(
            otel.tracer(env!("CARGO_PKG_NAME")),
        ))
        .with(cors))
}
