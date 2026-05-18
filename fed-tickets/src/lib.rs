use std::collections::HashMap;

use poem::{Endpoint, Route};
use poem_openapi::OpenApiService;
use sqlx::PgPool;

pub mod activities;
pub mod context;
pub mod groups;
pub mod healthcheck;
pub use context::Context;
use sqlx::types::Json;

pub type DbInternationalizedString = Json<InternationalizedString>;
pub type InternationalizedString = HashMap<String, String>;

/// # Errors
///
/// See [`Context::new`].
pub async fn get_endpoint(test_db: Option<PgPool>) -> color_eyre::Result<impl Endpoint> {
    let context = Context::new(test_db).await?;
    let api_service = OpenApiService::new(
        (
            activities::Router {
                context: context.clone(),
            },
            groups::Router {
                context: context.clone(),
            },
            healthcheck::Router {
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

    Ok(Route::new()
        .nest("/v0", api_service)
        .nest("/v0/docs", ui)
        .nest("/v0/spec.json", spec))
}
