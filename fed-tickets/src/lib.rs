use poem::{Endpoint, Route};
use poem_openapi::OpenApiService;
use sqlx::PgPool;

pub mod activities;
pub mod context;
pub mod groups;
pub mod healthcheck;
pub use context::Context;

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

    Ok(Route::new().nest("/v0", api_service).nest("/v0/docs", ui))
}
