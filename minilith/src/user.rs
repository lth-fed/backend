use std::ops::Deref;
use std::str::FromStr as _;

use fed_auth_verifier::{User, callbacks::AuthCallbackDataV1};
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _};
use poem_openapi::payload::Response;
use poem_openapi::{Enum, Object, OpenApi, payload::Json};
use sqlx::postgres::types::PgLTree;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;
use tracing::warn;

use crate::context::ContextWrapper;
use crate::group::Path;
use crate::{
    DbInternationalizedString as DIS, InternationalizedString, MinilithErrorOptionExt as _,
    MinilithResult,
};

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
    /// Groups this user directly administers.
    admin_group_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, Enum, sqlx::Type)]
#[oai(rename_all = "lowercase")]
#[sqlx(type_name = "notification_level", rename_all = "lowercase")]
enum NotificationLevel {
    None,
    Personalized,
    All,
}

#[derive(Debug, Object)]
struct GroupSetting {
    group_id: Uuid,
    visible: bool,
    notification_level: NotificationLevel,
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
                logo.id = groups.logo_id
            where user_id = $1
                and deleted = false"#,
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
        .await?;

        let user = sqlx::query!("select * from users where id = ($1)", user.get_id())
            .fetch_optional(&self.db)
            .await?
            .wrap_err_internal(
                "Your user object doesn't exist. Try logging out and then in again.",
            )?;

        let admin_group_ids = sqlx::query_scalar!(
            "select group_id from group_adminships where user_id = $1 order by group_id",
            user.id,
        )
        .fetch_all(&self.db)
        .await?;

        Ok(Json(Me {
            id: user.id,
            name: self
                .decrypt_string(user.name)
                .wrap_err_encryption("USER_ME_NAME")?,
            language: self
                .decrypt_string(user.language)
                .wrap_err_encryption("USER_ME_LANG")?,
            creation: user.creation,
            groups,
            admin_group_ids,
        }))
    }

    /// Stores the user's preferred language. It must be non-empty & consist of "0-9a-z-".
    #[oai(path = "/language", method = "put")]
    async fn update_language(
        &self,
        user: User,
        Json(language): Json<String>,
    ) -> MinilithResult<()> {
        if language.is_empty()
            || language.len() > 35
            || !language
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "invalid language: too long or empty or invalid characters",
                "language",
            ));
        }

        let encrypted_language = self.encrypt(&language);
        sqlx::query!(
            "update users set language = $1 where id = $2",
            encrypted_language,
            user.get_id(),
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Lists the user's explicit group filter settings. Groups without a row
    /// inherit from their nearest configured ancestor.
    #[oai(path = "/group-settings", method = "get")]
    async fn group_settings(&self, user: User) -> MinilithResult<Json<Vec<GroupSetting>>> {
        let settings = sqlx::query_as!(
            GroupSetting,
            r#"select
                group_id,
                visible,
                notification_level as "notification_level!: NotificationLevel"
            from user_group_settings
            where user_id = $1
            order by group_id"#,
            user.get_id(),
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(settings))
    }

    /// Creates or replaces an explicit group filter setting.
    #[oai(path = "/group-settings", method = "put")]
    #[allow(trivial_casts, reason = "sqlx custom enum parameter type override")]
    async fn update_group_setting(
        &self,
        user: User,
        Json(body): Json<GroupSetting>,
    ) -> MinilithResult<()> {
        sqlx::query_as!(
            GroupSetting,
            "insert into user_group_settings
                (user_id, group_id, visible, notification_level)
            values ($1, $2, $3, $4)
            on conflict (group_id, user_id) do update set
                visible = excluded.visible,
                notification_level = excluded.notification_level",
            user.get_id(),
            body.group_id,
            body.visible,
            body.notification_level as NotificationLevel,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
    /// # Errors
    ///
    /// DB, PARAM.
    #[oai(path = "/auth-callback/v1", method = "post")]
    async fn auth_callback_v1(&self, cb_data: AuthCallbackDataV1) -> MinilithResult<Response<()>> {
        if let Some(name) = cb_data.full_name {
            let name = self.encrypt(&name);

            sqlx::query!(
                "insert into users (id, name, language)
                    values ($1, $2, $3) on conflict (id) do update
                        set name = excluded.name",
                cb_data.sub,
                name,
                self.encrypt("")
            )
            .execute(&self.db)
            .await?;

            if let Some(guild) = cb_data.lth_guild {
                let guild_path = PgLTree::from_str(&format!("tlth.{}", guild.to_str()))
                    .wrap_err_internal("l1: failed to get ltree path from guild")?;
                // membership
                let gid = sqlx::query_scalar!(
                    "insert into group_memberships (user_id, group_id)
                    select $1 as user_id, id as group_id 
                    from groups where path = $2
                    -- so even if it exists we return it
                    on conflict (user_id, group_id) do update set group_id = excluded.group_id
                    returning group_id",
                    cb_data.sub,
                    guild_path
                )
                .fetch_one(&self.db)
                .await?;
                // visible + notifications from guild
                sqlx::query!(
                    "insert into user_group_settings (user_id, group_id,
                        visible, notification_level)
                    values ($1, $2, true, 'all'::notification_level)
                    on conflict do nothing",
                    cb_data.sub,
                    gid
                )
                .execute(&self.db)
                .await?;
                // visible - notifications from tlth
                sqlx::query!(
                    "insert into user_group_settings (user_id, group_id,
                        visible, notification_level)
                    select $1 as user_id, id as group_id, true as visible,
                    'none'::notification_level as notification_level
                    from groups where path = 'tlth'
                    on conflict do nothing",
                    cb_data.sub,
                )
                .execute(&self.db)
                .await?;
            } else {
                warn!(user_id = cb_data.sub, "User signed up without guild");
            }

            Ok(Response::new(()))
        } else {
            let has_user = sqlx::query_scalar!(
                "select exists (
                    select 1 from users where id = $1
                ) as \"exists!\"",
                cb_data.sub
            )
            .fetch_one(&self.db)
            .await?;
            if !has_user {
                return Ok(Response::new(()).status(poem::http::StatusCode::CREATED));
            }
            Ok(Response::new(()))
        }
    }
}
