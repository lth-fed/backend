use std::ops::Deref;

use fed_auth_verifier::CallbackDataV1;
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

#[OpenApi(prefix_path = "/user")]
impl Router {
    #[oai(path = "/auth-callback/v1", method = "post")]
    async fn auth_callback_v1(&self, cb_data: CallbackDataV1) -> poem::Result<()> {
        let nonce: [u8; 12] = rand::random();
        // this means we're leaking the name's length & lang's length, but I'm (Erik Davisson) is
        // pretty sure that's fine.
        let mut name: Vec<u8> = cb_data.full_name.into();
        self.endecrypt_mut_slice(&mut name, &nonce);

        sqlx::query!(
            "insert into users (id, name, language, nonce) values ($1, $2, $3, $4)",
            cb_data.sub,
            name,
            &[],
            &nonce
        )
        .execute(&self.db)
        .await
        // todo: use InternalServerError struct from lib.rs when that's merged
        .map_err(|_| {
            poem::Error::from_string("db failed", poem::http::StatusCode::INTERNAL_SERVER_ERROR)
        })?;
        Ok(())
    }
}
