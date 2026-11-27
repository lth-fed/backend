//! Voting (`evote`-style) API.
//!
//! Votes are deliberately not tied to users: clients generate a random UUID and
//! show it as a QR code, which an admin registers for the meeting. In secure
//! meetings the same registration also records the user so one person cannot
//! vote twice; the cast vote itself only references the client id.

#![allow(
    clippy::module_name_repetitions,
    reason = "`Vote` prefixes keep the public DTOs unambiguous"
)]

use std::ops::Deref;

use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use poem_openapi::param::{Path, Query};
use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi};
use uuid::Uuid;

use crate::context::ContextWrapper;
use crate::group::admin::check_direct_adminship;
use crate::{
    DbInternationalizedString as DIS, InternationalizedString as IS, MinilithEndpointError,
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

#[derive(Clone, Debug)]
pub struct AdminRouter {
    pub context: ContextWrapper,
}

impl Deref for AdminRouter {
    type Target = ContextWrapper;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Debug, Object)]
pub struct VoteMeeting {
    pub id: Uuid,
    pub group_id: Uuid,
    pub activity_id: Option<Uuid>,
    pub title: IS,
    pub secure_voting: bool,
}

#[derive(Debug, Object)]
pub struct VoteAlternative {
    pub id: Uuid,
    pub title: IS,
    pub voted_for: i32,
}

#[derive(Debug, Object)]
pub struct VoteRound {
    pub id: Uuid,
    pub title: IS,
    pub open: bool,
    pub max_choices: i32,
    pub alternatives: Vec<VoteAlternative>,
    /// `None` when no `client_id` was supplied.
    pub has_voted: Option<bool>,
}

#[derive(Debug, Object)]
pub struct VoteMeetingState {
    #[oai(flatten)]
    pub meeting: VoteMeeting,
    pub round: Option<VoteRound>,
}

#[derive(Debug, Object)]
pub struct CastVoteRequest {
    pub round_id: Uuid,
    /// The client-generated UUID that was registered by an admin.
    pub client_id: Uuid,
    pub alternative_ids: Vec<Uuid>,
}

#[derive(Debug, Object)]
pub struct CreateVoteMeeting {
    pub group_id: Uuid,
    pub activity_id: Option<Uuid>,
    pub secure_voting: bool,
    pub title: IS,
}

#[derive(Debug, Object)]
pub struct CreateVoteRound {
    pub title: IS,
    /// How many alternatives a client may select.
    pub max_choices: i32,
    pub alternatives: Vec<IS>,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct PutVoteRoundOpen {
    pub round_id: Uuid,
    pub open: bool,
}

#[derive(Debug, Object)]
pub struct RegisterVoteClient {
    pub client_id: Uuid,
    /// Required for secure meetings: records that this user has used their vote.
    pub user_id: Option<String>,
}

async fn meeting_visible(
    db: &bin_common::PgPool,
    user_id: &str,
    meeting_id: Uuid,
) -> MinilithResult<VoteMeeting> {
    sqlx::query!(
        r#"select meeting.id, meeting.group_id, meeting.activity_id,
            meeting.title as "title!: DIS", meeting.secure_voting
        from vote_meetings meeting
        where meeting.id = $1
        and (
            exists (
                select 1
                from group_memberships membership
                where membership.user_id = $2
                and membership.group_id = meeting.group_id
            )
            or exists (
                select 1 from vote_meeting_admins admin
                where admin.meeting_id = meeting.id
                and admin.user_id = $2
            )
            or exists (
                select 1 from group_adminships admin
                where admin.group_id = meeting.group_id and admin.user_id = $2
            )
        )"#,
        meeting_id,
        user_id,
    )
    .fetch_optional(db)
    .await?
    .map(|row| VoteMeeting {
        id: row.id,
        group_id: row.group_id,
        activity_id: row.activity_id,
        title: row.title.0,
        secure_voting: row.secure_voting,
    })
    .ok_or_else(|| MinilithEndpointError::unauthorized("no access to this vote meeting", ""))
}

/// Admins of the group or of the meeting may manage rounds.
async fn meeting_mutable(
    db: &bin_common::PgPool,
    user_id: &str,
    meeting_id: Uuid,
) -> MinilithResult<Uuid> {
    let meeting = meeting_visible(db, user_id, meeting_id).await?;
    let meeting_admin = sqlx::query_scalar!(
        r#"select readonly from vote_meeting_admins
        where meeting_id = $1 and user_id = $2"#,
        meeting_id,
        user_id,
    )
    .fetch_optional(db)
    .await?;
    if meeting_admin == Some(true) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "you may only view this meeting",
            "",
        ));
    }
    if meeting_admin.is_some() {
        return Ok(meeting.group_id);
    }
    check_direct_adminship(db, user_id, meeting.group_id).await?;
    Ok(meeting.group_id)
}

/// Poll `GET /v1/vote-meetings/{id}` **every 5 seconds** while a round is open.
/// The response carries the current round and live per-alternative counts.
#[OpenApi(prefix_path = "/v1")]
impl Router {
    /// Meetings the caller can see.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/vote-meetings", method = "get")]
    async fn meetings(&self, user: User) -> MinilithResult<Json<Vec<VoteMeeting>>> {
        let meetings = sqlx::query!(
            r#"select meeting.id, meeting.group_id, meeting.activity_id,
                meeting.title as "title!: DIS", meeting.secure_voting
            from vote_meetings meeting
            where exists (
                select 1
                from group_memberships membership
                inner join groups member_group on member_group.id = membership.group_id
                inner join groups meeting_group on meeting_group.id = meeting.group_id
                where membership.user_id = $1
                and member_group.path @> meeting_group.path
            )
            or exists (
                select 1 from vote_meeting_admins admin
                where admin.meeting_id = meeting.id
                and admin.user_id = $1
            )
            order by meeting.title->>'en'"#,
            user.get_id(),
        )
        .map(|row| VoteMeeting {
            id: row.id,
            group_id: row.group_id,
            activity_id: row.activity_id,
            title: row.title.0,
            secure_voting: row.secure_voting,
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(meetings))
    }

    /// The meeting's current open round. Poll this every 5 seconds.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/vote-meetings/:id", method = "get")]
    async fn meeting(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        client_id: Query<Option<Uuid>>,
    ) -> MinilithResult<Json<VoteMeetingState>> {
        let meeting = meeting_visible(&self.db, user.get_id(), id).await?;
        let Some(round) = sqlx::query!(
            r#"select id, title as "title!: DIS", open, max_choises
            from vote_rounds
            where meeting_id = $1 and open
            limit 1"#,
            id,
        )
        .fetch_optional(&self.db)
        .await?
        else {
            return Ok(Json(VoteMeetingState {
                meeting,
                round: None,
            }));
        };
        let alternatives = sqlx::query!(
            r#"select id, title as "title!: DIS", voted_for
            from vote_alternatives
            where round_id = $1
            order by idx"#,
            round.id,
        )
        .map(|row| VoteAlternative {
            id: row.id,
            title: row.title.0,
            voted_for: row.voted_for,
        })
        .fetch_all(&self.db)
        .await?;
        let has_voted = match client_id.0 {
            Some(client_id) => Some(
                sqlx::query_scalar!(
                    r#"select exists (
                        select 1 from vote_cast_votes
                        where client_id = $1 and round_id = $2
                    ) as "exists!""#,
                    client_id,
                    round.id,
                )
                .fetch_one(&self.db)
                .await?,
            ),
            None => None,
        };
        Ok(Json(VoteMeetingState {
            meeting,
            round: Some(VoteRound {
                id: round.id,
                title: round.title.0,
                open: round.open,
                max_choices: round.max_choises,
                alternatives,
                has_voted,
            }),
        }))
    }

    /// Cast (or change?) one vote. Votes are final; poll the meeting for results.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/vote-meetings/:id/votes", method = "put")]
    async fn cast_vote(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<CastVoteRequest>,
    ) -> MinilithResult<()> {
        let meeting = meeting_visible(&self.db, user.get_id(), id).await?;
        let mut txn = self.db.begin().await?;
        let round = sqlx::query!(
            "select max_choises, open from vote_rounds where id = $1 and meeting_id = $2",
            body.round_id,
            id,
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .wrap_err_not_found()?;
        if !round.open {
            return Err(MinilithEndpointError::bad_frontend_code(
                "the round is closed",
                "",
            ));
        }
        if body.alternative_ids.is_empty()
            || body.alternative_ids.len() > usize::try_from(round.max_choises).unwrap_or(0)
        {
            return Err(MinilithEndpointError::bad_user_input(
                "vote",
                "",
                "select at most `max_choices` alternatives",
                "alternative_ids",
            ));
        }
        if meeting.secure_voting {
            let registered = sqlx::query_scalar!(
                r#"select exists (
                    select 1 from vote_registered_clients
                    where client_id = $1 and meeting_id = $2
                ) as "exists!""#,
                body.client_id,
                id,
            )
            .fetch_one(&mut txn.executor())
            .await?;
            if !registered {
                return Err(MinilithEndpointError::unauthorized(
                    "this device is not registered for the meeting",
                    "",
                ));
            }
        } else {
            sqlx::query!(
                "insert into vote_registered_clients (client_id, meeting_id)
                values ($1, $2) on conflict do nothing",
                body.client_id,
                id,
            )
            .execute(&mut txn.executor())
            .await?;
        }
        let valid = sqlx::query_scalar!(
            r#"select count(*)::int as "count!"
            from vote_alternatives
            where round_id = $1 and id = any($2)"#,
            body.round_id,
            &body.alternative_ids,
        )
        .fetch_one(&mut txn.executor())
        .await?;
        if usize::try_from(valid).unwrap_or(0) != body.alternative_ids.len() {
            return Err(MinilithEndpointError::bad_user_input(
                "vote",
                "",
                "unknown alternative",
                "alternative_ids",
            ));
        }
        sqlx::query!(
            "insert into vote_cast_votes (client_id, round_id) values ($1, $2)",
            body.client_id,
            body.round_id,
        )
        .execute(&mut txn.executor())
        .await
        .map_err(|err| {
            if err
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
            {
                MinilithEndpointError::bad_frontend_code("you have already voted", "")
            } else {
                err.into()
            }
        })?;
        sqlx::query!(
            "update vote_alternatives set voted_for = voted_for + 1
            where round_id = $1 and id = any($2)",
            body.round_id,
            &body.alternative_ids,
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(())
    }
}

#[derive(Debug, Object)]
pub struct VoteMeetingAdmin {
    pub user_id: String,
    pub readonly: bool,
}

#[derive(Debug, Object)]
pub struct PutVoteMeetingAdmin {
    pub user_id: String,
    pub readonly: bool,
}

#[derive(Debug, Object)]
pub struct AdminVoteMeeting {
    #[oai(flatten)]
    pub meeting: VoteMeeting,
    /// False for Adjusters (read-only meeting admins), who may view but not change the meeting.
    pub can_edit: bool,
    pub rounds: Vec<VoteRound>,
}

#[OpenApi(prefix_path = "/v0/admin")]
impl AdminRouter {
    /// Meetings of groups the caller directly administers.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/vote-meetings", method = "get")]
    async fn admin_meetings(
        &self,
        user: User,
        Query(group_id): Query<Option<Uuid>>,
    ) -> MinilithResult<Json<Vec<VoteMeeting>>> {
        let meetings = sqlx::query!(
            r#"select
                meeting.id,
                meeting.group_id,
                meeting.activity_id,
                meeting.title as "title!: DIS",
                meeting.secure_voting
            from vote_meetings meeting
            where ($2::uuid is null or meeting.group_id = $2)
            and (exists (select 1 from group_adminships admin
                where admin.group_id = meeting.group_id and admin.user_id = $1)
                or exists (select 1 from vote_meeting_admins admin
                where admin.meeting_id = meeting.id and admin.user_id = $1))
            order by meeting.title::text"#,
            user.get_id(),
            group_id,
        )
        .map(|row| VoteMeeting {
            id: row.id,
            group_id: row.group_id,
            activity_id: row.activity_id,
            title: row.title.0,
            secure_voting: row.secure_voting,
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(meetings))
    }

    /// Admins of a meeting.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/vote-meetings/:id/admins", method = "get")]
    async fn meeting_admins(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<VoteMeetingAdmin>>> {
        // Anyone who can view the admin meeting can see its roles.
        self.admin_meeting(user, Path(id)).await?;
        let admins = sqlx::query!(
            "select user_id, readonly from vote_meeting_admins where meeting_id = $1 order by user_id",
            id,
        )
        .map(|row| VoteMeetingAdmin {
            user_id: row.user_id,
            readonly: row.readonly,
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(admins))
    }

    /// Adds or updates a meeting admin.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/vote-meetings/:id/admins", method = "put")]
    async fn put_meeting_admin(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<PutVoteMeetingAdmin>,
    ) -> MinilithResult<()> {
        let group_id = sqlx::query_scalar!("select group_id from vote_meetings where id = $1", id)
            .fetch_optional(&self.db)
            .await?
            .wrap_err_not_found()?;
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            "insert into vote_meeting_admins (user_id, meeting_id, readonly)
            values ($1, $2, $3)
            on conflict (user_id, meeting_id) do update set readonly = excluded.readonly",
            body.user_id,
            id,
            body.readonly,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Removes a meeting admin.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/vote-meetings/:id/admins/:user_id", method = "delete")]
    async fn delete_meeting_admin(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Path(user_id): Path<String>,
    ) -> MinilithResult<()> {
        let group_id = sqlx::query_scalar!("select group_id from vote_meetings where id = $1", id)
            .fetch_optional(&self.db)
            .await?
            .wrap_err_not_found()?;
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        sqlx::query!(
            "delete from vote_meeting_admins where meeting_id = $1 and user_id = $2",
            id,
            user_id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// A meeting with all of its rounds and live vote counts.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/vote-meetings/:id", method = "get")]
    async fn admin_meeting(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<AdminVoteMeeting>> {
        let meeting = sqlx::query!(
            r#"select
                meeting.id,
                meeting.group_id,
                meeting.activity_id,
                meeting.title as "title!: DIS",
                meeting.secure_voting
            from vote_meetings meeting
            where meeting.id = $1
            and (
                exists (
                    select 1 from group_adminships admin
                    where admin.group_id = meeting.group_id and admin.user_id = $2
                )
                or exists (
                    select 1 from vote_meeting_admins admin
                    where admin.meeting_id = meeting.id and admin.user_id = $2
                )
            )"#,
            id,
            user.get_id(),
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;
        let meeting = VoteMeeting {
            id: meeting.id,
            group_id: meeting.group_id,
            activity_id: meeting.activity_id,
            title: meeting.title.0,
            secure_voting: meeting.secure_voting,
        };
        let rounds = sqlx::query!(
            r#"select id, title as "title!: DIS", open, max_choises
            from vote_rounds where meeting_id = $1"#,
            id,
        )
        .fetch_all(&self.db)
        .await?;
        let mut result = Vec::with_capacity(rounds.len());
        for round in rounds {
            let alternatives = sqlx::query!(
                r#"select id, title as "title!: DIS", voted_for
                from vote_alternatives where round_id = $1 order by idx"#,
                round.id,
            )
            .map(|row| VoteAlternative {
                id: row.id,
                title: row.title.0,
                voted_for: row.voted_for,
            })
            .fetch_all(&self.db)
            .await?;
            result.push(VoteRound {
                id: round.id,
                title: round.title.0,
                open: round.open,
                max_choices: round.max_choises,
                alternatives,
                has_voted: None,
            });
        }
        let can_edit = meeting_mutable(&self.db, user.get_id(), id).await.is_ok();
        Ok(Json(AdminVoteMeeting {
            meeting,
            can_edit,
            rounds: result,
        }))
    }
    /// Delete a meeting and all its rounds, votes and registrations.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/vote-meetings/:id", method = "delete")]
    async fn delete_meeting(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        meeting_mutable(&self.db, user.get_id(), id).await?;
        sqlx::query!("delete from vote_meetings where id = $1", id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Create a vote meeting. Members of the group can then vote in it.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/vote-meetings", method = "post")]
    async fn create_meeting(
        &self,
        user: User,
        Json(body): Json<CreateVoteMeeting>,
    ) -> MinilithResult<Json<Uuid>> {
        check_direct_adminship(&self.db, user.get_id(), body.group_id).await?;
        if let Some(activity_id) = body.activity_id {
            crate::group::admin::check_activity_adminship(&self.db, user.get_id(), activity_id)
                .await?;
        }
        let id = Uuid::new_v4();
        let mut txn = self.db.begin().await?;
        sqlx::query!(
            "insert into vote_meetings (id, group_id, activity_id, secure_voting, title)
            values ($1, $2, $3, $4, $5)",
            id,
            body.group_id,
            body.activity_id,
            body.secure_voting,
            body.title.to_json_value(),
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "insert into vote_meeting_admins (user_id, meeting_id, readonly)
            values ($1, $2, false)",
            user.get_id(),
            id,
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(Json(id))
    }

    /// Open a new round with its alternatives.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/vote-meetings/:id/rounds", method = "post")]
    async fn create_round(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<CreateVoteRound>,
    ) -> MinilithResult<Json<Uuid>> {
        meeting_mutable(&self.db, user.get_id(), id).await?;
        if body.max_choices <= 0 || body.alternatives.is_empty() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "a round needs alternatives and max_choices > 0",
                "",
            ));
        }
        let round_id = Uuid::new_v4();
        let mut txn = self.db.begin().await?;
        sqlx::query!(
            "insert into vote_rounds
                (id, meeting_id, title, secret, max_choises, open)
            values ($1, $2, $3, $4, $5, true)",
            round_id,
            id,
            body.title.to_json_value(),
            Uuid::new_v4().to_string(),
            body.max_choices,
        )
        .execute(&mut txn.executor())
        .await?;
        for (idx, alternative) in body.alternatives.into_iter().enumerate() {
            sqlx::query!(
                "insert into vote_alternatives (id, round_id, idx, title, voted_for)
                values (uuidv4(), $1, $2, $3, 0)",
                round_id,
                i32::try_from(idx).unwrap_or(i32::MAX),
                alternative.to_json_value(),
            )
            .execute(&mut txn.executor())
            .await?;
        }
        txn.commit().await?;
        Ok(Json(round_id))
    }

    /// Open or close a round.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/vote-meetings/:id/rounds", method = "put")]
    async fn put_round_open(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<PutVoteRoundOpen>,
    ) -> MinilithResult<()> {
        meeting_mutable(&self.db, user.get_id(), id).await?;
        let result = sqlx::query!(
            "update vote_rounds set open = $3 where id = $1 and meeting_id = $2",
            body.round_id,
            id,
            body.open,
        )
        .execute(&self.db)
        .await?;
        if result.rows_affected() == 0 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "round not found",
                "",
            ));
        }
        Ok(())
    }

    /// Register a device (client-generated UUID QR code) for the meeting.
    ///
    /// For secure meetings, `user_id` is required and prevents that user from
    /// voting again, without linking the cast vote to them.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/vote-meetings/:id/clients", method = "post")]
    async fn register_client(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<RegisterVoteClient>,
    ) -> MinilithResult<()> {
        meeting_mutable(&self.db, user.get_id(), id).await?;
        let secure =
            sqlx::query_scalar!("select secure_voting from vote_meetings where id = $1", id)
                .fetch_one(&self.db)
                .await?;
        let mut txn = self.db.begin().await?;
        sqlx::query!(
            "insert into vote_registered_clients (client_id, meeting_id) values ($1, $2)",
            body.client_id,
            id,
        )
        .execute(&mut txn.executor())
        .await?;
        if secure {
            let Some(user_id) = body.user_id else {
                return Err(MinilithEndpointError::bad_user_input(
                    "user_id",
                    "",
                    "required for secure meetings",
                    "user_id",
                ));
            };
            sqlx::query!(
                "insert into vote_registered_users (user_id, meeting_id) values ($1, $2)",
                &user_id,
                id,
            )
            .execute(&mut txn.executor())
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }
}
