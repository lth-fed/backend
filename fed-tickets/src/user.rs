use std::ops::Deref;

use fed_auth_verifier::CallbackDataV1;
use poem_openapi::OpenApi;
use sqlx::postgres::types::PgLTree;

use crate::InternalServerError;
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
const DEMO: &str = include_str!("./demo.csv");
fn get_guild(stil_id: &str) -> Option<char> {
    for line in DEMO.lines().skip(1) {
        let Some(id) = line.split(',').nth(3) else {
            continue;
        };
        let id = id
            .strip_prefix('"')
            .unwrap_or(id)
            .strip_suffix('"')
            .unwrap_or(id);
        if id == stil_id {
            let guild = line
                .split(',')
                .nth(4)?
                .strip_prefix('"')
                .unwrap_or(id)
                .strip_suffix('"')
                .unwrap_or(id);
            return guild.chars().next().as_ref().map(char::to_ascii_lowercase);
        }
    }
    None
}

#[OpenApi(prefix_path = "/user")]
impl Router {
    async fn populate_tlth(&self) -> poem::Result<()> {
        sqlx::query!(
            "insert into images (id, size, url) values ('7c315a13-eff7-4268-89b9-5e072611ea21'::uuid, 0, 'https://icelk.dev/logo.png') on conflict do nothing",
        )
        .execute(&self.db)
        .await
        .map_err(InternalServerError::db)?;
        let paths = [
            "tlth",
            "tlth.f",
            "tlth.e",
            "tlth.m",
            "tlth.v",
            "tlth.a",
            "tlth.k",
            "tlth.d",
            "tlth.doct",
            "tlth.ing",
            "tlth.w",
            "tlth.i",
        ];
        for path in paths {
            sqlx::query!(
            "insert into groups (path, limit_membership_visibility, name, description, logo_id, deleted) values ($1, false, '{}'::jsonb, '{}'::jsonb, '7c315a13-eff7-4268-89b9-5e072611ea21'::uuid, false) on conflict do nothing",
            path.parse::<PgLTree>().map_err(InternalServerError::db)?
        )
        .execute(&self.db)
        .await
        .map_err(InternalServerError::db)?;
        }
        Ok(())
    }
    #[oai(path = "/auth-callback/v1", method = "post")]
    async fn auth_callback_v1(&self, cb_data: CallbackDataV1) -> poem::Result<()> {
        let nonce: [u8; 12] = rand::random();
        // this means we're leaking the name's length & lang's length, but I'm (Erik Davisson) is
        // pretty sure that's fine.
        let mut name: Vec<u8> = cb_data.full_name.into();
        self.endecrypt_mut_slice(&mut name, &nonce);

        sqlx::query!(
            "insert into users (id, name, language, nonce) values ($1, $2, $3, $4) on conflict do nothing",
            cb_data.sub,
            name,
            &[],
            &nonce
        )
        .execute(&self.db)
        .await
        .map_err(InternalServerError::db)?;

        if let Some(guild) = get_guild(cb_data.sub.strip_prefix("test:").unwrap_or(&cb_data.sub)) {
            self.populate_tlth().await?;
            let id = sqlx::query!(
                "select id from groups where groups.path = $1",
                format!("tlth.{guild}")
                    .parse::<PgLTree>()
                    .map_err(InternalServerError::db)?
            )
            .fetch_one(&self.db)
            .await
            .map_err(InternalServerError::db)?;
            let id = id.id;
            sqlx::query!(
                "insert into group_memberships (group_id, user_id) values ($1, $2) on conflict do nothing",
                id,
                cb_data.sub
            )
            .execute(&self.db)
            .await
            .map_err(InternalServerError::db)?;
        }

        Ok(())
    }
}
