use poem_openapi::{ApiResponse, OpenApi, payload::Json};

use crate::context::Context;

#[derive(Clone, Debug)]
pub struct Router {
    pub context: Context,
}

#[derive(Debug, ApiResponse)]
pub enum HealthCheck {
    #[oai(status = 200)]
    Ok(Json<String>),
    #[oai(status = 503)]
    Err(Json<String>),
}

#[OpenApi]
impl Router {
    #[oai(path = "/healthcheck", method = "get")]
    async fn health_check(&self) -> HealthCheck {
        match sqlx::query("SELECT 1").fetch_one(&self.context.db).await {
            Ok(_) => HealthCheck::Ok(Json("Ok :)".to_owned())),
            Err(error_or_something_idk) => {
                let timeout_secs = self.context.db.options().get_acquire_timeout().as_secs();
                tracing::error!(
                    "Health check database failure: '{error_or_something_idk}' with timeout {timeout_secs}s"
                );
                HealthCheck::Err(Json(format!(
                    "Service Unavailable: Database connection failed with timeout set to {timeout_secs}s",
                )))
            }
        }
    }
}
