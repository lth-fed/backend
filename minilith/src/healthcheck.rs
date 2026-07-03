use std::ops::Deref;

use minilith_errors::{MinilithErrorResultExt as _, MinilithResult};
use poem_openapi::{OpenApi, payload::PlainText};

use crate::context::Context;

#[derive(Clone, Debug)]
pub struct Router {
    pub context: Context,
}
impl Deref for Router {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[OpenApi]
impl Router {
    #[oai(path = "/healthcheck", method = "get")]
    async fn health_check(&self) -> MinilithResult<PlainText<String>> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.db)
            .await
            .wrap_err_db()?;
        Ok(PlainText("Ok :)".to_owned()))
    }
}
