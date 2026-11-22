//! Notification tag APIs: activity tags and news tags, including per-user opt-outs.
//!
//! Tags are owned by the group that created them. A user sees the tags of every group that is an
//! ancestor of (or equal to) one of their direct memberships, i.e. "current group and all parents".

#![allow(
    clippy::module_name_repetitions,
    reason = "`ActivityTag`/`NewsTag` keep the public DTOs unambiguous"
)]

use std::ops::Deref;

use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi};
use uuid::Uuid;

use crate::context::ContextWrapper;
use crate::{DbInternationalizedString as DIS, InternationalizedString as IS, MinilithResult};

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

#[derive(Debug, Clone, Object)]
pub struct AppTag {
    pub id: Uuid,
    pub title: IS,
    pub creator_id: Uuid,
    pub creator_path: String,
    /// Whether the caller has opted out of notifications for this tag.
    pub opted_out: bool,
}

const ACTIVITY_TAG_VISIBLE: &str = "exists (
    select 1
    from group_memberships membership
    inner join groups member_group on member_group.id = membership.group_id
    inner join groups creator on creator.id = tag.creator_id
    where membership.user_id = $1
    and creator.path @> member_group.path
)";

const NEWS_TAG_VISIBLE: &str = "exists (
    select 1
    from group_memberships membership
    inner join groups member_group on member_group.id = membership.group_id
    inner join groups creator on creator.id = tag.creator_id
    where membership.user_id = $1
    and creator.path @> member_group.path
)";

#[OpenApi(prefix_path = "/v1")]
impl Router {
    /// Activity tags the caller can opt out of.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/activity-tags", method = "get")]
    async fn activity_tags(&self, user: User) -> MinilithResult<Json<Vec<AppTag>>> {
        let query = format!(
            r#"select tag.id, tag.title as "title!: DIS", tag.creator_id,
                creator.path::text as "creator_path!",
                exists (
                    select 1 from activity_opt_out_tags opt_out
                    where opt_out.user_id = $1 and opt_out.tag_id = tag.id
                ) as "opted_out!"
            from activity_tags tag
            inner join groups creator on creator.id = tag.creator_id
            where {ACTIVITY_TAG_VISIBLE}
            order by tag.title->>'en'"#
        );
        let tags = sqlx::query(&query)
            .bind(user.get_id())
            .map(|row: sqlx::postgres::PgRow| {
                use sqlx::Row as _;
                AppTag {
                    id: row.get("id"),
                    title: row.get::<DIS, _>("title").0,
                    creator_id: row.get("creator_id"),
                    creator_path: row.get("creator_path"),
                    opted_out: row.get("opted_out"),
                }
            })
            .fetch_all(&self.db)
            .await?;
        Ok(Json(tags))
    }

    /// Opt out of notifications for an activity tag.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/activity-tags/:id/opt-out", method = "put")]
    async fn opt_out_activity_tag(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        let query = format!(
            "select {ACTIVITY_TAG_VISIBLE} as \"visible!\" from activity_tags tag where tag.id = $2"
        );
        let visible = sqlx::query_scalar::<_, bool>(&query)
            .bind(user.get_id())
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .wrap_err_not_found()?;
        if !visible {
            return Err(minilith_errors::MinilithEndpointError::unauthorized(
                "unknown tag",
                "",
            ));
        }
        sqlx::query!(
            "insert into activity_opt_out_tags (user_id, tag_id) values ($1, $2)
            on conflict do nothing",
            user.get_id(),
            id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Start receiving notifications for an activity tag again.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/activity-tags/:id/opt-out", method = "delete")]
    async fn opt_in_activity_tag(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        sqlx::query!(
            "delete from activity_opt_out_tags where user_id = $1 and tag_id = $2",
            user.get_id(),
            id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// News tags the caller can opt out of.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/news-tags", method = "get")]
    async fn news_tags(&self, user: User) -> MinilithResult<Json<Vec<AppTag>>> {
        let query = format!(
            r#"select tag.id, tag.title as "title!: DIS", tag.creator_id,
                creator.path::text as "creator_path!",
                exists (
                    select 1 from news_push_opt_out_tags opt_out
                    where opt_out.user_id = $1 and opt_out.tag_id = tag.id
                ) as "opted_out!"
            from news_tags tag
            inner join groups creator on creator.id = tag.creator_id
            where {NEWS_TAG_VISIBLE}
            order by tag.title->>'en'"#
        );
        let tags = sqlx::query(&query)
            .bind(user.get_id())
            .map(|row: sqlx::postgres::PgRow| {
                use sqlx::Row as _;
                AppTag {
                    id: row.get("id"),
                    title: row.get::<DIS, _>("title").0,
                    creator_id: row.get("creator_id"),
                    creator_path: row.get("creator_path"),
                    opted_out: row.get("opted_out"),
                }
            })
            .fetch_all(&self.db)
            .await?;
        Ok(Json(tags))
    }

    /// Opt out of push notifications for a news tag.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/news-tags/:id/opt-out", method = "put")]
    async fn opt_out_news_tag(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        let query = format!(
            "select {NEWS_TAG_VISIBLE} as \"visible!\" from news_tags tag where tag.id = $2"
        );
        let visible = sqlx::query_scalar::<_, bool>(&query)
            .bind(user.get_id())
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .wrap_err_not_found()?;
        if !visible {
            return Err(minilith_errors::MinilithEndpointError::unauthorized(
                "unknown tag",
                "",
            ));
        }
        sqlx::query!(
            "insert into news_push_opt_out_tags (user_id, tag_id) values ($1, $2)
            on conflict do nothing",
            user.get_id(),
            id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Start receiving push notifications for a news tag again.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/news-tags/:id/opt-out", method = "delete")]
    async fn opt_in_news_tag(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        sqlx::query!(
            "delete from news_push_opt_out_tags where user_id = $1 and tag_id = $2",
            user.get_id(),
            id,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
}
