use std::collections::HashMap;
use std::fmt::Display;

use fed_auth_verifier::AuthContext;
use poem::http::{Method, StatusCode};
use poem::middleware::Cors;
use poem::{Endpoint, EndpointExt as _, Route};
use poem_openapi::OpenApiService;
use sqlx::PgPool;
use tracing::error;

pub mod activities;
pub mod context;
pub mod group;
pub mod healthcheck;
pub mod ticket;
pub mod user;

pub use context::Context;
use sqlx::types::Json;

pub type DbInternationalizedString = Json<InternationalizedString>;
#[derive(Debug, Clone, poem_openapi::NewType, serde::Serialize, serde::Deserialize)] // eventually implement Deserialize ourselves
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

#[derive(Debug)]
pub struct InternalServerError<E: std::error::Error + Send + Sync + 'static> {
    inner: E,
}
impl<E: Display + std::error::Error + Send + Sync + 'static> InternalServerError<E> {
    #[track_caller]
    pub fn generic(error: E) -> Self {
        error!("Internal server error: {error}");
        Self { inner: error }
    }
    #[track_caller]
    pub fn db(error: E) -> Self {
        error!("Database error: {error}");
        Self { inner: error }
    }
}
impl InternalServerError<poem::Error> {
    #[track_caller]
    pub fn encryption(error: &str) -> Self {
        error!("Encryption error error: {error}");
        Self {
            inner: poem::Error::from_string(error.to_owned(), StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}
impl<E: std::error::Error + Send + Sync + 'static> From<InternalServerError<E>> for poem::Error {
    fn from(value: InternalServerError<E>) -> Self {
        poem::error::InternalServerError(value.inner)
    }
}

/// # Errors
///
/// See [`Context::new`].
pub async fn get_endpoint(test_db: Option<PgPool>) -> color_eyre::Result<impl Endpoint> {
    let context = Context::new(test_db).await?;
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
        .nest("/v0", api_service.data(auth_context))
        .nest("/v0/docs", ui)
        .nest("/v0/spec.json", spec)
        .with(cors))
}
