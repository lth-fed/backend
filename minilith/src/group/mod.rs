use std::ops::Deref;

use fed_auth_verifier::User;
use poem_openapi::{
    ApiResponse, Object, OpenApi, param,
    payload::{Json, PlainText},
};
use sqlx::PgExecutor;
use uuid::Uuid;

pub mod admin;
pub mod member;
mod path;

pub use path::Path;

use crate::{
    DbInternationalizedString as DIS, InternationalizedString as IS, MinilithEndpointError,
    MinilithErrorOptionExt as _, MinilithErrorResultExt as _, MinilithResult,
};
use crate::{
    context::ContextWrapper,
    group::{
        admin::{Adminship, check_adminship, check_parent_adminship, group_admins},
        member::{group_members, user_groups},
    },
};

/// Looks up a group's uuid by its path.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn id_by_path(db: impl PgExecutor<'_>, path: &Path) -> MinilithResult<Option<Uuid>> {
    sqlx::query_scalar!("select id from groups where path = $1", path.0)
        .fetch_optional(db)
        .await
        .wrap_err_db()
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
}

#[derive(Debug, Object)]
pub struct CreateGroupRequest {
    pub path: Path,
    pub name: IS,
    pub description: IS,
    pub limit_membership_visibility: bool,
}

#[derive(Debug, Object)]
pub struct CreateAdminship {
    /// The ID of the user to make an admin.
    pub user_id: String,
}

#[derive(Debug, ApiResponse)]
pub enum CreateGroupResponse {
    #[oai(status = 201)]
    Ok(Json<Group>),
}

#[derive(Debug, ApiResponse)]
pub enum RemoveAdminshipResponse {
    #[oai(status = 200)]
    Ok(PlainText<&'static str>),
}

#[OpenApi]
impl Router {
    /// List all groups the user is a direct or transitive member of.
    ///
    /// # Errors
    ///
    /// DB, AUTH
    #[oai(path = "/groups", method = "get")]
    async fn list_groups(&self, user: User) -> MinilithResult<Json<Vec<Group>>> {
        let groups = user_groups(&self.context.db, user.get_id()).await?;

        Ok(Json(groups))
    }

    /// Creates a new group under the given parent group.
    ///
    /// The user performing this action must be an admin of the parent group.
    ///
    /// # Errors
    ///
    /// - group has no parent
    /// - group is root, you can't create root group
    /// - group already exists
    #[oai(path = "/groups", method = "post")]
    async fn create_group(
        &self,
        user: User,
        Json(create_group): Json<CreateGroupRequest>,
    ) -> MinilithResult<CreateGroupResponse> {
        let mut txn = self.db.begin().await.wrap_err_db()?;

        let CreateGroupRequest {
            path,
            name,
            description,
            limit_membership_visibility,
        } = create_group;

        let parent = path
            .parent()
            .wrap_err_bad_frontend("group has to have a parent; you can't create a root group")?;
        let parent_id = id_by_path(&mut txn.executor(), &parent)
            .await?
            .wrap_err_bad_frontend("parent group doesn't exist")?;
        check_adminship(&mut txn.executor(), user.get_id(), parent_id).await?;

        let group = sqlx::query_as!(
            Group,
            r#"insert into groups (path, name, description, limit_membership_visibility)
            values ($1, $2, $3, $4)
            returning
                id, path, limit_membership_visibility, name as "name!: DIS",
                description as "description!: DIS", deleted"#,
            path.0,
            name.to_json_value(),
            description.to_json_value(),
            limit_membership_visibility,
        )
        .fetch_one(&mut txn.executor())
        .await
        .map_err(|err| match err {
            sqlx::Error::Database(ref db_err) if let Some(constraint) = db_err.constraint() => {
                match constraint {
                    "groups_path_key" => MinilithEndpointError::bad_frontend_code(
                        "GRP_EXISTS",
                        "a group with the same path/id already exists",
                    ),
                    "groups_parent_path_fkey" => MinilithEndpointError::bad_frontend_code(
                        "GRP_NULL_PARENT",
                        "no parent with path `{parent}` exists",
                    ),
                    _unknown_constraint => MinilithEndpointError::db(err),
                }
            }
            other_err => MinilithEndpointError::db(other_err),
        })?;

        txn.commit().await.wrap_err_db()?;

        Ok(CreateGroupResponse::Ok(Json(group)))
    }

    /// List all members of a group. To do it, you need to be an admin of the
    /// group.
    ///
    /// # Errors
    ///
    /// - none
    #[oai(path = "/groups/:group_id/members", method = "get")]
    async fn list_members(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<Json<Vec<String>>> {
        let mut txn = self.db.begin().await.wrap_err_db()?;
        check_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let members = group_members(&mut txn.executor(), group_id).await?;

        Ok(Json(members))
    }

    /// List all admins of a group. To do it, you need to be an admin of the
    /// group.
    ///
    /// # Errors
    ///
    /// - none
    #[oai(path = "/groups/:group_id/admins", method = "get")]
    async fn list_admins(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<Json<Vec<String>>> {
        let mut txn = self.db.begin().await.wrap_err_db()?;
        check_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let admins = group_admins(&mut txn.executor(), group_id).await?;

        Ok(Json(admins))
    }

    /// Create an adminship for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    ///
    /// # Errors
    ///
    /// - root must have no admins
    /// - the user must be admin of the parent of this group
    #[oai(path = "/groups/:group_id/admins", method = "post")]
    async fn create_adminship(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        Json(create_adminship): Json<CreateAdminship>,
    ) -> MinilithResult<Json<Adminship>> {
        let CreateAdminship { user_id } = create_adminship;

        let mut txn = self.db.begin().await.wrap_err_db()?;

        check_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let adminship = admin::create_adminship(&mut txn.executor(), &user_id, group_id).await?;
        // todo: notify other admins via email
        txn.commit().await.wrap_err_db()?;

        Ok(Json(adminship))
    }

    /// Removes an adminship for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    ///
    /// # Errors
    ///
    /// - root must have no admins
    /// - the user must be admin of the parent of this group
    #[oai(path = "/groups/:group_id/admins/:user_id", method = "delete")]
    async fn remove_adminship(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        Json(user_id): Json<String>,
    ) -> MinilithResult<RemoveAdminshipResponse> {
        let mut txn = self.db.begin().await.wrap_err_db()?;

        check_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        admin::remove_adminship(&mut txn.executor(), &user_id, group_id).await?;
        // todo: notify other admins via email
        txn.commit().await.wrap_err_db()?;

        Ok(RemoveAdminshipResponse::Ok(PlainText("adminship removed")))
    }
}
