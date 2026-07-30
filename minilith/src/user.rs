use std::ops::Deref;

use fed_auth_verifier::{User, callbacks::AuthCallbackDataV1};
use poem_openapi::{Enum, Object, OpenApi, param::Path as ApiPath, payload::Json};
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;

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

#[derive(Debug, Object)]
struct UpdateGroupSetting {
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
        .await?;

        let user = sqlx::query!("select * from users where id = ($1)", user.get_id())
            .fetch_optional(&self.db)
            .await?
            .wrap_err_internal(
                "Your user object doesn't exist. Try logging out and then in again.",
            )?;

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
    #[oai(path = "/group-settings/:group_id", method = "put")]
    #[allow(trivial_casts, reason = "sqlx custom enum parameter type override")]
    async fn update_group_setting(
        &self,
        user: User,
        ApiPath(group_id): ApiPath<Uuid>,
        Json(body): Json<UpdateGroupSetting>,
    ) -> MinilithResult<Json<GroupSetting>> {
        let in_user_tree = sqlx::query_scalar!(
            r#"select exists (
                select 1
                from groups target
                inner join group_memberships membership on membership.user_id = $1
                inner join groups member_group on member_group.id = membership.group_id
                where target.id = $2
                and subpath(target.path, 0, 1) = subpath(member_group.path, 0, 1)
            ) as "exists!""#,
            user.get_id(),
            group_id,
        )
        .fetch_one(&self.db)
        .await?;
        if !in_user_tree {
            return Err(crate::MinilithEndpointError::bad_frontend_code(
                "group is outside the user's group trees",
                "",
            ));
        }

        let setting = sqlx::query_as!(
            GroupSetting,
            r#"insert into user_group_settings
                (user_id, group_id, visible, notification_level)
            values ($1, $2, $3, $4)
            on conflict (group_id, user_id) do update set
                visible = excluded.visible,
                notification_level = excluded.notification_level
            returning
                group_id,
                visible,
                notification_level as "notification_level!: NotificationLevel""#,
            user.get_id(),
            group_id,
            body.visible,
            body.notification_level as NotificationLevel,
        )
        .fetch_one(&self.db)
        .await?;
        Ok(Json(setting))
    }
    /// # Errors
    ///
    /// DB, PARAM.
    #[oai(path = "/auth-callback/v1", method = "post")]
    async fn auth_callback_v1(&self, cb_data: AuthCallbackDataV1) -> MinilithResult<()> {
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
        .await?;

        Ok(())
    }
}
