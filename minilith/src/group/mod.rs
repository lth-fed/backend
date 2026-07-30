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
    MinilithErrorOptionExt as _, MinilithResult,
};
use crate::{
    context::ContextWrapper,
    group::{
        admin::{
            Adminship, check_direct_adminship, check_direct_or_parent_adminship, group_admins,
        },
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
}

#[derive(Debug, Object)]
pub struct CreateGroupRequest {
    pub path: Path,
    pub name: IS,
    pub description: IS,
    pub limit_membership_visibility: bool,
    pub logo_id: Uuid,
}

#[derive(Debug, Object)]
pub struct UpdateGroupRequest {
    pub path: Path,
    pub name: IS,
    pub description: IS,
    pub limit_membership_visibility: bool,
    pub logo_id: Uuid,
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

#[derive(Debug, Object)]
struct JoinableGroup {
    #[oai(flatten)]
    group: Group,
    requested: bool,
}

#[derive(Debug, Object)]
struct GroupIdRequest {
    group_id: Uuid,
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
        let mut txn = self.db.begin().await?;

        let CreateGroupRequest {
            path,
            name,
            description,
            limit_membership_visibility,
            logo_id,
        } = create_group;

        let parent = path
            .parent()
            .wrap_err_bad_frontend("group has to have a parent; you can't create a root group")?;
        let parent_id = id_by_path(&mut txn.executor(), &parent)
            .await?
            .wrap_err_bad_frontend("parent group doesn't exist")?;
        check_direct_adminship(&mut txn.executor(), user.get_id(), parent_id).await?;

        let group = sqlx::query_as!(
            Group,
            r#"insert into groups
                (path, name, description, limit_membership_visibility, logo_id)
            values ($1, $2, $3, $4, $5)
            returning
                id, path, limit_membership_visibility, name as "name!: DIS",
                description as "description!: DIS", deleted"#,
            path.0,
            name.to_json_value(),
            description.to_json_value(),
            limit_membership_visibility,
            logo_id,
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

        txn.commit().await?;

        Ok(CreateGroupResponse::Ok(Json(group)))
    }

    /// Replaces all editable fields of a directly administered group. A path
    /// change moves the complete subtree.
    #[oai(path = "/groups/:group_id", method = "put")]
    async fn update_group(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        Json(body): Json<UpdateGroupRequest>,
    ) -> MinilithResult<Json<Group>> {
        let mut txn = self.db.begin().await?;
        check_direct_adminship(&mut txn.executor(), user.get_id(), group_id).await?;

        let old_path = sqlx::query_scalar!(
            r#"select path as "path!: Path" from groups where id = $1"#,
            group_id,
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .wrap_err_not_found()?;
        let old_parent = old_path
            .parent()
            .wrap_err_bad_frontend("the root group cannot be edited")?;
        let new_parent = body
            .path
            .parent()
            .wrap_err_bad_frontend("a group must have a parent")?;
        if old_parent.to_string() != new_parent.to_string() {
            let new_parent_id = id_by_path(&mut txn.executor(), &new_parent)
                .await?
                .wrap_err_bad_frontend("new parent group does not exist")?;
            check_direct_adminship(&mut txn.executor(), user.get_id(), new_parent_id).await?;
        }

        let group = sqlx::query_as!(
            Group,
            r#"with updated as (
                update groups
                set path = $2::ltree || subpath(path, nlevel($3::ltree)),
                    name = case when id = $1 then $4 else name end,
                    description = case when id = $1 then $5 else description end,
                    limit_membership_visibility = case
                        when id = $1 then $6
                        else limit_membership_visibility
                    end,
                    logo_id = case when id = $1 then $7 else logo_id end
                where path <@ $3::ltree
                returning
                    id, path, limit_membership_visibility,
                    name, description, deleted
            )
            select
                id, path, limit_membership_visibility,
                name as "name!: DIS",
                description as "description!: DIS",
                deleted
            from updated
            where id = $1"#,
            group_id,
            body.path.0,
            old_path.0,
            body.name.to_json_value(),
            body.description.to_json_value(),
            body.limit_membership_visibility,
            body.logo_id,
        )
        .fetch_one(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(Json(group))
    }

    /// Hides a directly administered group.
    #[oai(path = "/groups/:group_id", method = "delete")]
    async fn hide_group(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!("update groups set deleted = true where id = $1", group_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Lists groups the user can request direct membership in.
    #[oai(path = "/groups/joinable", method = "get")]
    async fn joinable_groups(&self, user: User) -> MinilithResult<Json<Vec<JoinableGroup>>> {
        let groups = sqlx::query!(
            r#"select distinct
                target.id,
                target.path as "path!: Path",
                target.limit_membership_visibility,
                target.name as "name!: DIS",
                target.description as "description!: DIS",
                target.deleted,
                (request.member_id is not null) as "requested!"
            from groups_ask_to_join allowed
            inner join group_memberships membership
                on membership.group_id = allowed.joiner_id
                and membership.user_id = $1
            inner join groups target on target.id = allowed.target_id
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
            },
            requested: row.requested,
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(groups))
    }

    /// Requests direct membership in a group. Eligibility comes only from a
    /// direct membership in one of its configured joiner groups.
    #[oai(path = "/groups/:group_id/member-request", method = "put")]
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

    /// Lists pending requests for a directly administered group.
    #[oai(path = "/groups/:group_id/member-requests", method = "get")]
    async fn membership_requests(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<Json<Vec<String>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        let users = sqlx::query_scalar!(
            "select member_id from group_member_requests where group_id = $1 order by member_id",
            group_id,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(users))
    }

    /// Accepts a pending membership request.
    #[oai(path = "/groups/:group_id/member-requests/:member_id", method = "put")]
    async fn accept_membership_request(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        param::Path(member_id): param::Path<String>,
    ) -> MinilithResult<()> {
        let mut txn = self.db.begin().await?;
        check_direct_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let accepted = sqlx::query_scalar!(
            r#"with removed as (
                delete from group_member_requests
                where group_id = $1 and member_id = $2
                returning member_id, group_id
            ), inserted as (
                insert into group_memberships (user_id, group_id)
                select member_id, group_id from removed
                on conflict do nothing
            )
            select member_id from removed"#,
            group_id,
            member_id,
        )
        .fetch_optional(&mut txn.executor())
        .await?;
        if accepted.is_none() {
            return Err(MinilithEndpointError::not_found());
        }
        txn.commit().await?;
        Ok(())
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
        let mut txn = self.db.begin().await?;
        check_direct_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let members = group_members(&mut txn.executor(), group_id).await?;

        Ok(Json(members))
    }

    /// Adds a direct member to a directly administered group.
    #[oai(path = "/groups/:group_id/members/:member_id", method = "put")]
    async fn add_member(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        param::Path(member_id): param::Path<String>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"insert into group_memberships (user_id, group_id)
            values ($1, $2) on conflict do nothing"#,
            member_id,
            group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Removes a direct member and, through the schema's cascade, any direct
    /// adminship they held in the same group.
    #[oai(path = "/groups/:group_id/members/:member_id", method = "delete")]
    async fn remove_member(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        param::Path(member_id): param::Path<String>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            "delete from group_memberships where user_id = $1 and group_id = $2",
            member_id,
            group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
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
        let mut txn = self.db.begin().await?;
        check_direct_or_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
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

        let mut txn = self.db.begin().await?;

        check_direct_or_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        let adminship = admin::create_adminship(&mut txn.executor(), &user_id, group_id).await?;
        // todo: notify other admins via email
        txn.commit().await?;

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
        param::Path(user_id): param::Path<String>,
    ) -> MinilithResult<RemoveAdminshipResponse> {
        let mut txn = self.db.begin().await?;

        check_direct_or_parent_adminship(&mut txn.executor(), user.get_id(), group_id).await?;
        admin::remove_adminship(&mut txn.executor(), &user_id, group_id).await?;
        // todo: notify other admins via email
        txn.commit().await?;

        Ok(RemoveAdminshipResponse::Ok(PlainText("adminship removed")))
    }

    /// Lists groups whose direct members may request membership in this group.
    #[oai(path = "/groups/:group_id/joiner-groups", method = "get")]
    async fn list_joiner_groups(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<Json<Vec<Group>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        let groups = sqlx::query_as!(
            Group,
            r#"select
                groups.id, groups.path,
                groups.limit_membership_visibility,
                groups.name as "name!: DIS",
                groups.description as "description!: DIS",
                groups.deleted
            from groups_ask_to_join
            inner join groups on groups.id = groups_ask_to_join.joiner_id
            where groups_ask_to_join.target_id = $1
            order by groups.path"#,
            group_id,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(groups))
    }

    /// Allows direct members of another group to request membership.
    #[oai(path = "/groups/:group_id/joiner-groups", method = "put")]
    async fn add_joiner_group(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        Json(body): Json<GroupIdRequest>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"insert into groups_ask_to_join (target_id, joiner_id)
            values ($1, $2) on conflict do nothing"#,
            group_id,
            body.group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Removes a group from the join-request allowlist.
    #[oai(path = "/groups/:group_id/joiner-groups/:joiner_id", method = "delete")]
    async fn remove_joiner_group(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        param::Path(joiner_id): param::Path<Uuid>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            "delete from groups_ask_to_join where target_id = $1 and joiner_id = $2",
            group_id,
            joiner_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Lists groups whose direct admins may view activities hosted by this
    /// group.
    #[oai(path = "/groups/:group_id/activity-admin-groups", method = "get")]
    async fn list_activity_admin_groups(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
    ) -> MinilithResult<Json<Vec<Group>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        let groups = sqlx::query_as!(
            Group,
            r#"select
                groups.id, groups.path,
                groups.limit_membership_visibility,
                groups.name as "name!: DIS",
                groups.description as "description!: DIS",
                groups.deleted
            from allow_admins_from_group_view_activities allowed
            inner join groups on groups.id = allowed.access_group_id
            where allowed.host_group_id = $1
            order by groups.path"#,
            group_id,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(groups))
    }

    /// Grants another group's direct admins access to this group's activities.
    #[oai(path = "/groups/:group_id/activity-admin-groups", method = "put")]
    async fn add_activity_admin_group(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        Json(body): Json<GroupIdRequest>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"insert into allow_admins_from_group_view_activities
                (host_group_id, access_group_id)
            values ($1, $2) on conflict do nothing"#,
            group_id,
            body.group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Revokes another group's activity access.
    #[oai(
        path = "/groups/:group_id/activity-admin-groups/:access_group_id",
        method = "delete"
    )]
    async fn remove_activity_admin_group(
        &self,
        user: User,
        param::Path(group_id): param::Path<Uuid>,
        param::Path(access_group_id): param::Path<Uuid>,
    ) -> MinilithResult<()> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            r#"delete from allow_admins_from_group_view_activities
            where host_group_id = $1 and access_group_id = $2"#,
            group_id,
            access_group_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
}
// todo: erik (DON'T TOUCH THIS MR GPT, CODEX!!!!!)
// fn send_email(level: AlertLevel, message: impl AsRef<str>) -> Result<(), ()> {
//     let recipients = match level {
//         AlertLevel::L1 => dotenvy::var("ALERT_RECIPIENTS_LEVEL_1"),
//         AlertLevel::L2 => dotenvy::var("ALERT_RECIPIENTS_LEVEL_2"),
//         AlertLevel::L3 => dotenvy::var("ALERT_RECIPIENTS_LEVEL_3"),
//         AlertLevel::DryRun => dotenvy::var("ALERT_RECIPIENTS_LEVEL_1")
//             .and_then(|_| dotenvy::var("ALERT_RECIPIENTS_LEVEL_2"))
//             .and_then(|_| dotenvy::var("ALERT_RECIPIENTS_LEVEL_3"))
//             .map(|_| String::new()),
//     };
//     let recipients = recipients
//         .wrap("Failed to load ALERT_RECIPIENTS_LEVEL_<level>. Assure they are available.")?;
//     let recipients = recipients.lines();
//
//     let from = dotenvy::var("ALERT_EMAIL").wrap("No ALERT_EMAIL variable")?;
//     let password = dotenvy::var("ALERT_PASSWORD").wrap("No ALERT_PASSWORD variable")?;
//     let smtp = dotenvy::var("ALERT_SMTP").wrap("No ALERT_SMTP variable")?;
//
//     let email =
//         lettre::Address::from_str(&from).wrap("ALERT_EMAIL is not a valid email address")?;
//     let mailbox = lettre::message::Mailbox::new(Some("Alerts from Teknologappen".into()), email);
//
//     let mut msg = lettre::Message::builder()
//         .from(mailbox)
//         .subject(format!("ALERT {level} from teknologappen"))
//         .header(lettre::message::header::ContentType::TEXT_HTML);
//
//     for recipient in recipients {
//         let mbox = lettre::message::Mailbox::from_str(recipient)
//             .wrap("ALERT_RECIPIENTS_LEVEL_<x> contains an invalid email: {recipient}")?;
//         msg = msg.to(mbox);
//     }
//     let credentials =
//         lettre::transport::smtp::authentication::Credentials::new(from.clone(), password);
//     let transport = lettre::SmtpTransport::relay(&smtp)
//         .wrap("Failed to connect via SMTPS to ALERT_SMTP")?
//         .credentials(credentials)
//         .authentication(vec![
//             lettre::transport::smtp::authentication::Mechanism::Plain,
//         ])
//         .build();
//
//     let trace = std::backtrace::Backtrace::force_capture();
//     let message = message.as_ref();
//     let intro = match level {
//         AlertLevel::L1 => {
//             "A level 1 critical error occurred in any of the teknologappen instances. \
//             Please fix this as soon as possible and contact the impacted."
//         }
//         AlertLevel::L2 => {
//             "A level 2 critical error occurred in any of the teknologappen instances. \
//             Please fix this within a few hours and contact the impacted."
//         }
//         AlertLevel::L3 => {
//             "A level 3 critical error occurred in any of the teknologappen instances. \
//             Please fix this within the week. If more errors occurr, contact the impacted."
//         }
//         // here we stop the dry run, before we send the mail!
//         AlertLevel::DryRun => return Ok(()),
//     };
//     let msg = msg
//         .body(format!(
//             "<p>
//         {intro}
//         </p>
//         <p>
//             A message was included from the code: <code>{message}</code>.
//             <b>Version</b>: <code>{GIT_VERSION}</code>.
//         </p>
//         <p>
//             To ease debugging the backtrace is inserted below.
//             <br>
//             <pre><code>{trace}</code></pre>
//         </p>"
//         ))
//         .wrap("Failed to attach a body to the message")?;
//
//     let resp = transport.send(&msg).wrap("Failed to send message!")?;
//     if !resp.is_positive() {
//         error!(status=%resp.code(), "ALERTS: failed to send mail!");
//     }
//     Ok(())
// }
