use std::ops::Deref;
use std::sync::Arc;

use bin_common::{PgPool, get_otel};
use minilith_errors::MinilithEndpointError;
use opentelemetry::trace::TracerProvider as _;
use poem::http::Method;
use poem::middleware::{Cors, OpenTelemetryMetrics, OpenTelemetryTracing};
use poem::{Endpoint, EndpointExt as _, Route};
use poem_openapi::auth::Bearer;
use poem_openapi::{Enum, Object, OpenApiService, SecurityScheme};
use serde::{Deserialize, Serialize};

pub use fed_auth_verifier::callbacks::{
    TransactionCallbackInfo as CallbackInfo, TransactionInfo, TransactionState,
};

use self::context::Context;

pub mod api;
pub mod callback;
pub mod context;
pub mod receipt;
pub mod runtime;
pub mod swish;

#[derive(sqlx::Type, Debug, Clone, Copy, Serialize, Enum)]
#[sqlx(rename_all = "lowercase")]
#[oai(rename_all = "lowercase")]
pub enum Provider {
    Swish,
    Stripe,
    Free,
}

#[derive(Serialize, Deserialize, Debug, Object, Clone)]
pub struct CallbackEvent {
    callback_url_v1: String,
    client_id: String,
    // it's a bit ugly the other types are not in this crate but then we avoid duplicated code.
    #[serde(flatten)]
    #[oai(flatten)]
    inner: CallbackInfo,
}

/// HAS to be registered as [`poem::EndpointExt::data`].
#[derive(Debug, Clone)]
pub struct ApiAuthContext {
    db: PgPool,
}

#[derive(Debug)]
pub struct ApiAuthData {
    pub client_id: String,
    pub token: String,
    pub callback_url_v1: String,
}
#[derive(SecurityScheme)]
#[oai(ty = "bearer", checker = "ApiAuth::from_token")]
#[derive(Debug)]
pub struct ApiAuth(ApiAuthData);
impl ApiAuth {
    async fn from_token(req: &poem::Request, token: Bearer) -> poem::Result<ApiAuthData> {
        let context: &ApiAuthContext = req.data().ok_or_else(|| {
            MinilithEndpointError::internal_error("ApiAuthContext not registered as data!", "")
        })?;

        let Some(row) = sqlx::query_as!(
            ApiAuthData,
            "select token, client_id, callback_url_v1 from api_tokens where token = $1",
            &token.token
        )
        .fetch_optional(&context.db)
        .await
        .map_err(MinilithEndpointError::from)?
        else {
            return Err(MinilithEndpointError::unauthorized("token invalid", "").into());
        };
        Ok(row)
    }
}
impl Deref for ApiAuth {
    type Target = ApiAuthData;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// # Errors
///
/// See [`Context::new`].
pub async fn get_endpoint(test_db: Option<PgPool>) -> color_eyre::Result<impl Endpoint> {
    let otel = get_otel(env!("CARGO_PKG_NAME"), test_db.is_some())?;

    let context = Arc::new(Context::new(test_db).await?);
    let api_auth_context = ApiAuthContext {
        db: context.db.clone(),
    };

    if cfg!(not(test)) {
        if let Err(err) = runtime::initial_checks(&context).await {
            tracing::error!(?err, "failed to run initial checks!");
            return Err(color_eyre::Report::msg(""));
        }
        runtime::spawn(&context);
    }

    let api_service = OpenApiService::new(
        api::Route { context },
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    // this url is just for the Swagger UI
    .server("http://localhost:8002/v0");
    let ui = api_service.swagger_ui();
    let spec = api_service.spec_endpoint();

    let cors = Cors::new()
        .allow_method(Method::GET)
        .allow_method(Method::POST)
        .allow_header("content-type")
        .allow_header("authorization")
        .allow_credentials(true);

    Ok(Route::new()
        .nest("/v0", api_service.data(api_auth_context))
        .nest("/v0/docs", ui)
        .nest("/v0/spec.json", spec)
        .with(OpenTelemetryMetrics::new())
        .with(OpenTelemetryTracing::new(
            otel.tracer(env!("CARGO_PKG_NAME")),
        ))
        .with(cors))
}
