use fed_auth_verifier::User;
use poem::{Error, Result, error::InternalServerError, http::StatusCode};
use poem_openapi::{
    ApiResponse, Object, OpenApi, param,
    payload::{Json, PlainText},
};
use sqlx::postgres::types::PgLTreeLabel;

pub mod admin;
pub mod member;
mod path;

pub use path::Path;

use crate::{
    context::Context,
    group::{
        admin::{Adminship, check_adminship, check_parent_adminship, group_admins},
        member::{group_members, user_groups},
    },
};

#[derive(Clone, Debug)]
pub struct Router {
    pub context: Context,
}

#[derive(Debug, Object)]
pub struct Group {
    pub path: Path,
    pub limit_membership_visibility: bool,
    pub name: serde_json::Value,
    pub description: serde_json::Value,
    pub deleted: bool,
}

#[allow(
    clippy::module_name_repetitions,
    reason = "for uniformity in the generated OpenAPI schema"
)]
#[derive(Debug, Object)]
pub struct CreateGroup {
    /// The parent group's ID, if any.
    ///
    /// An `null` or empty path is equivalent to no parent.
    #[oai(default)]
    pub parent: Path,
    pub name: serde_json::Value,
    pub description: serde_json::Value,
    pub limit_membership_visibility: bool,
}

#[derive(Debug, Object)]
pub struct CreateAdminship {
    /// The ID of the user to make an admin.
    pub user_id: String,
}

#[derive(Debug, ApiResponse)]
pub enum ListGroupsResponse {
    #[oai(status = 200)]
    Ok(Json<Vec<Group>>),
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
    async fn list_groups(&self, user: User) -> Result<ListGroupsResponse> {
        let groups = user_groups(&self.context.db, user.get_id())
            .await
            .map_err(InternalServerError)?;

        Ok(ListGroupsResponse::Ok(Json(groups)))
    }

    /// Creates a new group under the given parent group.
    ///
    /// The user performing this action must be an admin of the parent group.
    #[oai(path = "/groups", method = "post")]
    async fn create_group(
        &self,
        user: User,
        Json(create_group): Json<CreateGroup>,
    ) -> Result<CreateGroupResponse> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;

        let CreateGroup {
            parent,
            name,
            description,
            limit_membership_visibility,
        } = create_group;

        check_adminship(&mut *txn, user.get_id(), &parent).await?;

        let label = generate_group_label(&name)
            .ok_or_else(|| Error::from_string("invalid name", StatusCode::BAD_REQUEST))?;

        let path = parent.join(label);

        let group = sqlx::query_as!(
            Group,
            "insert into groups (path, name, description, limit_membership_visibility) values ($1, $2, $3, $4) returning path, limit_membership_visibility, name, description, deleted",
            path.0,
            name,
            description,
            limit_membership_visibility,
        )
        .fetch_one(&mut *txn)
        .await
        .map_err(|err| match err {
            sqlx::Error::Database(ref db_err) if let Some(constraint) = db_err.constraint() => match constraint {
                "groups_pkey" => Error::from_string("group already exists", StatusCode::CONFLICT),
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
    #[oai(path = "/groups/:group/members", method = "get")]
    async fn list_members(
        &self,
        user: User,
        param::Path(group): param::Path<Path>,
    ) -> Result<Json<Vec<String>>> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;
        check_adminship(&mut *txn, user.get_id(), &group).await?;
        let members = group_members(&mut *txn, &group)
            .await
            .map_err(InternalServerError)?;

        Ok(Json(members))
    }

    /// List all admins of a group. To do it, you need to be an admin of the
    /// group.
    #[oai(path = "/groups/:group/admins", method = "get")]
    async fn list_admins(
        &self,
        user: User,
        param::Path(group): param::Path<Path>,
    ) -> Result<Json<Vec<String>>> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;
        check_adminship(&mut *txn, user.get_id(), &group).await?;
        let admins = group_admins(&mut *txn, &group)
            .await
            .map_err(InternalServerError)?;

        Ok(Json(admins))
    }

    /// Create an adminship for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    #[oai(path = "/groups/:group/admins", method = "post")]
    async fn create_adminship(
        &self,
        user: User,
        param::Path(group): param::Path<Path>,
        Json(create_adminship): Json<CreateAdminship>,
    ) -> Result<Json<Adminship>> {
        let CreateAdminship { user_id } = create_adminship;

        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;

        check_parent_adminship(&mut *txn, user.get_id(), &group).await?;
        let adminship = admin::create_adminship(&mut *txn, &user_id, &group)
            .await
            .map_err(InternalServerError)?;
        txn.commit().await.map_err(InternalServerError)?;

        Ok(Json(adminship))
    }

    /// Removes an adminship for a user in a group.
    ///
    /// The user performing this action must be a literal super-admin, meaning
    /// they must at least be an administrator of the parent group.
    #[oai(path = "/groups/:group/admins/:user_id", method = "delete")]
    async fn remove_adminship(
        &self,
        user: User,
        param::Path(group): param::Path<Path>,
        Json(user_id): Json<String>,
    ) -> Result<RemoveAdminshipResponse> {
        let mut txn = self.context.db.begin().await.map_err(InternalServerError)?;

        check_parent_adminship(&mut *txn, user.get_id(), &group).await?;
        admin::remove_adminship(&mut *txn, &user_id, &group)
            .await
            .map_err(InternalServerError)?;
        txn.commit().await.map_err(InternalServerError)?;

        Ok(RemoveAdminshipResponse::Ok(PlainText("adminship removed")))
    }
}

fn generate_group_label(value: &serde_json::Value) -> Option<PgLTreeLabel> {
    value.as_object()?.iter().find_map(|(_key, value)| {
        let value = value.as_str()?;
        PgLTreeLabel::new(value).ok()
    })
}
