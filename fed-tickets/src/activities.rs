use poem_openapi::OpenApi;

use crate::context::Context;

#[derive(Clone, Debug)]
pub struct Router {
    pub context: Context,
}

#[OpenApi]
impl Router {
    // ...
}
