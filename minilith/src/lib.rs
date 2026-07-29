use std::collections::HashMap;
use std::sync::Arc;

use bin_common::get_otel;
use fed_auth_verifier::{AuthUrl, JwkContext, TransactionsUrl};
use opentelemetry::trace::TracerProvider as _;
use poem::http::Method;
use poem::middleware::{Cors, OpenTelemetryMetrics, OpenTelemetryTracing};
use poem::{Endpoint, EndpointExt as _, Route};
use poem_openapi::OpenApiService;

pub mod activities;
pub mod context;
pub mod group;
pub mod healthcheck;
mod runtime;
pub mod ticket;
mod transactions;
pub mod user;

pub use bin_common::PgPool;
pub use context::{Context, ContextWrapper};
pub use minilith_errors::*;
use tracing::error;

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
    #[must_use]
    pub fn resolve_intl<'a>(&'a self, user_language: &str, default: &'a str) -> &'a str {
        if let Some(translation) = self.0.get(user_language) {
            return translation;
        }
        if let Some(translation) = self
            .0
            .get(user_language.split('-').next().unwrap_or(user_language))
        {
            return translation;
        }
        if let Some(translation) = self.0.get("en") {
            return translation;
        }
        if let Some(translation) = self.0.get("sv") {
            return translation;
        }
        if let Some(translation) = self.0.values().next() {
            return translation;
        }
        default
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
///
/// # Panics
///
/// If 1 >= 60.
pub async fn get_endpoint(
    test_db: Option<PgPool>,
    migrate: bool,
) -> color_eyre::Result<impl Endpoint> {
    let otel = get_otel(env!("CARGO_PKG_NAME"), test_db.is_some())?;

    let context = Arc::new(Context::new(test_db, migrate).await?);

    #[allow(
        clippy::cfg_not_test,
        reason = "it causes errors because the DB is closed before this can start up"
    )]
    #[cfg(not(test))]
    if let Err(err) = runtime::initial_checks(&context).await {
        error!(?err, "failed to run initial checks!");
        return Err(color_eyre::Report::msg(""));
    }

    #[allow(
        clippy::cfg_not_test,
        reason = "it causes errors because the DB is closed before this can start up"
    )]
    #[cfg(not(test))]
    runtime::spawn(&context);

    let auth_context = JwkContext::<AuthUrl>::new("teknologappen").await?;
    let transaction_context = JwkContext::<TransactionsUrl>::new("teknologappen").await?;
    let api_service = OpenApiService::new(
        (
            activities::Router {
                context: Arc::clone(&context),
            },
            group::Router {
                context: Arc::clone(&context),
            },
            ticket::Router {
                context: Arc::clone(&context),
            },
            healthcheck::Router {
                context: Arc::clone(&context),
            },
            user::Router {
                context: Arc::clone(&context),
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
        .allow_origin("https://teknologappen.se")
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        .allow_method(Method::PUT)
        .allow_method(Method::DELETE)
        .allow_header("content-type")
        .allow_header("authorization")
        .allow_credentials(true);

    Ok(Route::new()
        .nest(
            "/v0",
            api_service.data(auth_context).data(transaction_context),
        )
        .nest("/v0/docs", ui)
        .nest("/v0/spec.json", spec)
        .with(OpenTelemetryMetrics::new())
        .with(OpenTelemetryTracing::new(
            otel.tracer(env!("CARGO_PKG_NAME")),
        ))
        .with(cors))
}
