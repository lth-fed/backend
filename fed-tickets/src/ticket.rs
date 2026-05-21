use std::ops::Deref;

use fed_auth_verifier::User;
use poem_openapi::OpenApi;

use crate::Context;

#[derive(Debug, Clone)]
pub struct Router {
    pub context: Context,
}

impl Deref for Router {
    type Target = Context;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[OpenApi(prefix_path = "/tickets")]
impl Router {
    #[oai(path = "/", method = "get")]
    async fn my_tickets(&self, user: User) {
        todo!()
    }
}
