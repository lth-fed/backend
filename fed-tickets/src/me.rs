#![allow(clippy::unused_async, reason = "OpenAPI requires async handlers")]
use std::ops::Deref;

use crate::InternalServerError;
use fed_auth_verifier::User;
use poem_openapi::{ApiResponse, Object, OpenApi, payload::Json, payload::PlainText};
use sqlx::types::time::OffsetDateTime;

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

#[derive(Object)]
struct Me {
    id: String,
    name: String,
    language: String,
    latest_refresh: OffsetDateTime,
    creation: OffsetDateTime,
    inactive_since: Option<OffsetDateTime>,
}

#[OpenApi]
impl Router {
    #[oai(path = "/me", method = "get")]
    async fn me(&self, user: User) -> poem::Result<Json<Me>> {
        let query = sqlx::query!("select * from users where id = ($1)", user.get_id());
        let val = query
            .fetch_one(&self.db)
            .await
            .map_err(InternalServerError::db)?;
        Ok(Json(Me {
            id: val.id,
            name: self
                .decrypt_string(val.name, &val.nonce)
                .ok_or(InternalServerError::encryption("user.name"))?,
            language: self
                .decrypt_string(val.language, &val.nonce)
                .ok_or(InternalServerError::encryption("user.language"))?,
            latest_refresh: val.latest_refresh,
            creation: val.creation,
            inactive_since: val.inactive_since,
        }))
    }
}
