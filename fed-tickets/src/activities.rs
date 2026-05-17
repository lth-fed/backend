use std::ops::Deref;

use poem_openapi::OpenApi;

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
    // ...
}
