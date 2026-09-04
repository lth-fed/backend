use std::ops::Deref;

use minilith_errors::MinilithResult;
use poem_openapi::{Object, OpenApi, payload::Json};
use time::OffsetDateTime;

use crate::context::ContextWrapper;

#[derive(Clone, Debug)]
pub struct Router {
    pub context: ContextWrapper,
}
impl Deref for Router {
    type Target = ContextWrapper;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Object)]
struct Now {
    utc: OffsetDateTime,
}

#[OpenApi]
impl Router {
    #[oai(path = "/time", method = "get")]
    #[allow(clippy::unused_async, reason = "poem requires it")]
    async fn health_check(&self) -> MinilithResult<Json<Now>> {
        Ok(Json(Now {
            utc: OffsetDateTime::now_utc(),
        }))
    }
}
