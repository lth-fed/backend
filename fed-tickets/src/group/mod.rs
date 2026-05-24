use std::ops::Deref;

use fed_auth_verifier::User;
use poem::{Error, Result, error::InternalServerError, http::StatusCode};
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

use crate::{DbInternationalizedString as DIS, InternationalizedString as IS};
use crate::{
    context::Context,
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
pub async fn id_by_path(db: impl PgExecutor<'_>, path: &Path) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!("select id from groups where path = $1", path.0)
        .fetch_optional(db)
        .await
}

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
    #[oai(path = "/groups", method = "get")]
    async fn list_groups(&self, user: User) -> Result<Json<Vec<Group>>> {
        let groups = user_groups(&self.context.db, user.get_id())
            .await
            .map_err(InternalServerError)?;

        Ok(Json(groups))
    }

    /// Creates a new group under the given parent group.
    ///
    /// The user performing this action must be an admin of the parent group.
    #[oai(path = "/groups", method = "post")]
    async fn create_group(
        &self,
        user: User,
        Json(create_group): Json<CreateGroupRequest>,
    ) -> Result<CreateGroupResponse> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;

        let CreateGroupRequest {
            path,
            name,
            description,
            limit_membership_visibility,
        } = create_group;

        let parent = path
            .parent()
            .ok_or_else(|| Error::from_string("no parent group", StatusCode::BAD_REQUEST))?;
        let parent_id = id_by_path(&mut *txn, &parent)
            .await
            .map_err(InternalServerError)?
            .ok_or_else(|| {
                Error::from_string(
                    format!("no parent with path `{parent}` exists"),
                    StatusCode::BAD_REQUEST,
                )
            })?;
        check_adminship(&mut *txn, user.get_id(), parent_id).await?;

        let group = sqlx::query_as!(
            Group,
            r#"insert into groups (path, name, description, limit_membership_visibility) values ($1, $2, $3, $4) returning id, path, limit_membership_visibility, name as "name!: DIS", description as "description!: DIS", deleted"#,
            path.0,
            name.to_json_value(),
            description.to_json_value(),
            limit_membership_visibility,
        )
        .fetch_one(&mut *txn)
        .await
        .map_err(|err| match err {
            sqlx::Error::Database(ref db_err) if let Some(constraint) = db_err.constraint() => match constraint {
                "groups_path_key" => Error::from_string("group already exists", StatusCode::CONFLICT),
                "groups_parent_path_fkey" => Error::from_string(format!("no parent with path `{parent}` exists"), StatusCode::BAD_REQUEST),
                _unknown_constraint => InternalServerError(err),
            }
            other_err => InternalServerError(other_err),
        })?;

        txn.commit().await.map_err(InternalServerError)?;

        Ok(CreateGroupResponse::Ok(Json(group)))
    }

    /// List all members of a group. To do it, you need to be an admin of the
    /// group.
    #[oai(path = "/groups/:group_id/members", method = "get")]
    async fn list_members(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> Result<Json<Vec<String>>> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;
        check_adminship(&mut *txn, user.get_id(), group_id).await?;
        let members = group_members(&mut *txn, group_id)
            .await
            .map_err(InternalServerError)?;

        Ok(Json(members))
    }

    /// List all admins of a group. To do it, you need to be an admin of the
    /// group.
    #[oai(path = "/groups/:group_id/admins", method = "get")]
    async fn list_admins(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> Result<Json<Vec<String>>> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;
        check_adminship(&mut *txn, user.get_id(), group_id).await?;
        let admins = group_admins(&mut *txn, group_id)
            .await
            .map_err(InternalServerError)?;

        Ok(Json(admins))
    }

    /// Create an adminship for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    #[oai(path = "/groups/:group_id/admins", method = "post")]
    async fn create_adminship(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        Json(create_adminship): Json<CreateAdminship>,
    ) -> Result<Json<Adminship>> {
        let CreateAdminship { user_id } = create_adminship;

        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;

        check_parent_adminship(&mut *txn, user.get_id(), group_id).await?;
        let adminship = admin::create_adminship(&mut *txn, &user_id, group_id)
            .await
            .map_err(InternalServerError)?;
        // todo: notify other admins via email
        txn.commit().await.map_err(InternalServerError)?;

        Ok(Json(adminship))
    }

    /// Removes an adminship for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    #[oai(path = "/groups/:group_id/admins/:user_id", method = "delete")]
    async fn remove_adminship(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        Json(user_id): Json<String>,
    ) -> Result<RemoveAdminshipResponse> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;

        check_parent_adminship(&mut *txn, user.get_id(), group_id).await?;
        admin::remove_adminship(&mut *txn, &user_id, group_id)
            .await
            .map_err(InternalServerError)?;
        // todo: notify other admins via email
        txn.commit().await.map_err(InternalServerError)?;

        Ok(RemoveAdminshipResponse::Ok(PlainText("adminship removed")))
    }
}
