use std::ops::Deref;

use fed_auth_verifier::{CallbackDataV1, User};
use poem_openapi::{Object, OpenApi, payload::Json};
use sqlx::postgres::types::PgLTree;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;

use crate::context::Context;
use crate::group::Path;
use crate::{
    DbInternationalizedString as DIS, InternationalizedString, MinilithErrorOptionExt as _,
    MinilithErrorResultExt as _, MinilithResult,
};

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

#[derive(Object)]
struct MyGroup {
    id: Uuid,
    path: Path,
    name: InternationalizedString,
    description: InternationalizedString,
    logo_url: String,
}

#[derive(Object)]
struct Me {
    id: String,
    name: String,
    language: String,
    creation: OffsetDateTime,
    groups: Vec<MyGroup>,
}

#[OpenApi(prefix_path = "/user")]
impl Router {
    /// # Errors
    ///
    /// AUTH, DB, ENC.
    #[oai(path = "/", method = "get")]
    async fn me(&self, user: User) -> MinilithResult<Json<Me>> {
        let groups = sqlx::query!(
            r#"select
                groups.id, path as "path!: Path",
                name as "name!: DIS",
                description as "description!: DIS",
                url as logo_url from group_memberships 
            inner join groups on
                groups.id = group_memberships.group_id 
            inner join images logo on
                logo.id = groups.logo_id where user_id = $1"#,
            user.get_id()
        )
        .map(|group| MyGroup {
            id: group.id,
            path: group.path,
            name: group.name.0,
            description: group.description.0,
            logo_url: group.logo_url,
        })
        .fetch_all(&self.db)
        .await
        .wrap_err_db()?;

        let user = sqlx::query!("select * from users where id = ($1)", user.get_id())
            .fetch_one(&self.db)
            .await
            .wrap_err_db()?;

        Ok(Json(Me {
            id: user.id,
            name: self
                .decrypt_string(user.name, &user.nonce)
                .wrap_err_encryption("USER_ME_NAME")?,
            language: self
                .decrypt_string(user.language, &user.nonce)
                .wrap_err_encryption("USER_ME_LANG")?,
            creation: user.creation,
            groups,
        }))
    }
    /// # Errors
    ///
    /// DB, PARAM.
    #[oai(path = "/auth-callback/v1", method = "post")]
    async fn auth_callback_v1(&self, cb_data: CallbackDataV1) -> MinilithResult<()> {
        let nonce: [u8; 12] = rand::random();
        // this means we're leaking the name's length & lang's length, but I'm (Erik Davisson) is
        // pretty sure that's fine.
        let mut name: Vec<u8> = cb_data.full_name.into();
        self.endecrypt_mut_slice(&mut name, &nonce);

        sqlx::query!(
            "insert into users (id, name, language, nonce)
            values ($1, $2, $3, $4) on conflict do nothing",
            cb_data.sub,
            name,
            &[],
            &nonce
        )
        .execute(&self.db)
        .await
        .wrap_err_db()?;

        if let Some(guild) = get_guild(cb_data.sub.strip_prefix("test:").unwrap_or(&cb_data.sub)) {
            let id = sqlx::query!(
                "select id from groups where groups.path = $1",
                format!("tlth.{guild}").parse::<PgLTree>().wrap_err_db()?
            )
            .fetch_one(&self.db)
            .await
            .wrap_err_db()?;
            let id = id.id;
            sqlx::query!(
                "insert into group_memberships (group_id, user_id)
                values ($1, $2) on conflict do nothing",
                id,
                cb_data.sub
            )
            .execute(&self.db)
            .await
            .wrap_err_db()?;
        }

        Ok(())
    }
}
