use std::ops::Deref;

use fed_auth_verifier::User;
use poem_openapi::{Object, OpenApi, param, payload::Json};
use sqlx::PgExecutor;
use uuid::Uuid;

pub mod admin;
pub mod member;
mod path;

pub use path::Path;

use crate::context::ContextWrapper;
use crate::{
    DbInternationalizedString as DIS, InternationalizedString as IS, MinilithEndpointError,
    MinilithResult,
};

use self::member::user_groups_tree;

/// Looks up a group's uuid by its path.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn id_by_path(db: impl PgExecutor<'_>, path: &Path) -> MinilithResult<Option<Uuid>> {
    sqlx::query_scalar!("select id from groups where path = $1", path.0)
        .fetch_optional(db)
        .await
        .map_err(Into::into)
}

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

#[derive(Debug, Object)]
pub struct Group {
    pub id: Uuid,
    pub path: Path,
    pub limit_membership_visibility: bool,
    pub name: IS,
    pub description: IS,
    pub deleted: bool,
    pub logo_id: Uuid,
    pub logo_url: String,
}
#[derive(Debug, Object)]
#[allow(clippy::module_name_repetitions, reason = "ye")]
pub struct FatGroup {
    pub id: Uuid,
    pub path: Path,
    pub limit_membership_visibility: bool,
    pub name: IS,
    pub description: IS,
    pub deleted: bool,
    pub logo_id: Uuid,
    pub logo_url: String,
    pub admin_ids: Option<Vec<String>>,
}

#[derive(Debug, Object)]
struct JoinableGroup {
    #[oai(flatten)]
    group: Group,
    requested: bool,
}

#[OpenApi(prefix_path = "/groups")]
impl Router {
    /// List all groups the user is a direct or transitive member of.
    /// for groups.
    ///
    /// # Errors
    ///
    /// DB, AUTH
    #[oai(path = "/tree", method = "get")]
    async fn tree(&self, user: User) -> MinilithResult<Json<Vec<FatGroup>>> {
        let groups = user_groups_tree(&self.context.db, user.get_id()).await?;

        Ok(Json(groups))
    }

    /// List all groups the user has visibility/notification settings for and groups with events
    /// in the future or recent path.
    ///
    /// # Errors
    ///
    /// DB, AUTH
    #[oai(path = "/for-filters", method = "get")]
    async fn for_filters(&self, user: User) -> MinilithResult<Json<Vec<Group>>> {
        let groups = sqlx::query_as!(
            Group,
            r#"select distinct
                pg.id,
                pg.path,
                pg.limit_membership_visibility,
                pg.name as "name!: DIS",
                pg.description as "description!: DIS",
                pg.deleted,
                logo.id as logo_id,
                logo.url as logo_url
            from groups g
            join groups pg on pg.path @> g.path
            join images logo on logo.id = pg.logo_id
            where g.deleted = false
            and exists (
                select 1
                from group_memberships gm
                join groups mg on mg.id = gm.group_id
                where gm.user_id = $1
                and subpath(g.path, 0, 1) = subpath(mg.path, 0, 1)
            )
            and (
                exists (
                    select 1
                    from activity_hosts ah
                    join activities act on act.id = ah.activity_id
                    where ah.group_id = g.id
                    and act.time_start > now() - interval '3 months'
                )
                or exists (
                    select 1
                    from user_group_settings ugs
                    where ugs.group_id = g.id
                    and ugs.user_id = $1
                )
            )
            order by pg.path"#,
            user.get_id()
        )
        .fetch_all(&self.db)
        .await?;

        Ok(Json(groups))
    }

    /// Lists groups the user can request direct membership in.
    #[oai(path = "/joinable", method = "get")]
    async fn joinable_groups(&self, user: User) -> MinilithResult<Json<Vec<JoinableGroup>>> {
        let groups = sqlx::query!(
            r#"select distinct
                target.id,
                target.path as "path!: Path",
                target.limit_membership_visibility,
                target.name as "name!: DIS",
                target.description as "description!: DIS",
                target.deleted,
                (request.member_id is not null) as "requested!",
                logo.id as logo_id,
                logo.url as logo_url
            from groups_ask_to_join allowed
            inner join group_memberships membership
                on membership.group_id = allowed.joiner_id
                and membership.user_id = $1
            inner join groups target on target.id = allowed.target_id
            inner join images logo on logo.id = target.logo_id
            left join group_member_requests request
                on request.group_id = target.id and request.member_id = $1
            where target.deleted = false
            order by target.path"#,
            user.get_id(),
        )
        .map(|row| JoinableGroup {
            group: Group {
                id: row.id,
                path: row.path,
                limit_membership_visibility: row.limit_membership_visibility,
                name: row.name.0,
                description: row.description.0,
                deleted: row.deleted,
                logo_id: row.logo_id,
                logo_url: row.logo_url,
            },
            requested: row.requested,
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(groups))
    }

    /// Requests direct membership in a group. Eligibility comes only from a
    /// direct membership in one of its configured joiner groups.
    #[oai(path = "/:group_id/member-request", method = "put")]
    async fn request_membership(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<()> {
        let allowed = sqlx::query_scalar!(
            r#"select exists (
                select 1
                from groups_ask_to_join
                inner join group_memberships
                    on group_memberships.group_id = groups_ask_to_join.joiner_id
                where groups_ask_to_join.target_id = $2
                and group_memberships.user_id = $1
            ) as "exists!""#,
            user.get_id(),
            group_id,
        )
        .fetch_one(&self.db)
        .await?;
        if !allowed {
            return Err(MinilithEndpointError::bad_frontend_code(
                "not allowed to request membership in this group",
                "",
            ));
        }
        sqlx::query!(
            r#"insert into group_member_requests (member_id, group_id)
            values ($1, $2) on conflict do nothing"#,
            user.get_id(),
            group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
    /// # Errors
    ///
    /// - errors if the user is an admin of this group
    #[oai(path = "/:group_id", method = "delete")]
    async fn leave_group(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<()> {
        match sqlx::query!(
            "delete from group_memberships where user_id = $1 and group_id = $2",
            user.get_id(),
            group_id
        )
        .execute(&self.db)
        .await
        {
            Ok(res) if res.rows_affected() == 1 => return Ok(()),
            Ok(_) => return Err(MinilithEndpointError::not_found()),
            Err(err)
                if err
                    .as_database_error()
                    .is_some_and(sqlx::error::DatabaseError::is_foreign_key_violation) =>
            {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "cannot remove membership of admin",
                    "",
                ));
            }
            res => res.map(|_| ())?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test(fixtures("adminship"))]
    async fn adminship_email_reaches_target_and_parent_admins(db: PgPool) {
        sqlx::query!(
            r#"insert into users (id, name, language) values
                ('email:board@example.com', ''::bytea, ''::bytea),
                ('email:peer@example.com', ''::bytea, ''::bytea),
                ('email:new@example.com', ''::bytea, ''::bytea)"#,
        )
        .execute(&db)
        .await
        .unwrap();

        let e_id = id_by_path(&db, &"tlth.e".parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        let nolla_id = id_by_path(&db, &"tlth.e.nolla".parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        for user_id in ["email:user_a", "email:board@example.com"] {
            admin::create_adminship(&db, user_id, e_id).await.unwrap();
        }
        for user_id in ["email:peer@example.com", "email:new@example.com"] {
            admin::create_adminship(&db, user_id, nolla_id)
                .await
                .unwrap();
        }

        let recipients = crate::admin::change_admin_email_recipients(
            &db,
            nolla_id,
            "email:user_a",
            "email:new@example.com",
        )
        .await
        .unwrap()
        .into_iter()
        .map(|recipient| recipient.user_id)
        .collect::<Vec<_>>();
        assert_eq!(
            recipients,
            [
                "email:board@example.com",
                "email:new@example.com",
                "email:peer@example.com",
            ],
            "the acting parent admin is omitted, while another parent admin, the affected admin, \
            and existing target admins are notified",
        );
    }
}
