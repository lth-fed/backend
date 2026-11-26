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
use crate::{
    DbInternationalizedString as DIS, InternationalizedString as IS, MinilithResult, PgPool,
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

#[derive(Debug, Clone, Object)]
pub struct AppTag {
    pub id: Uuid,
    pub title: IS,
    pub creator_id: Uuid,
    pub creator_path: String,
    /// Whether the caller has opted out of notifications for this tag.
    pub opted_out: bool,
    /// False for tags everyone gets, such as "Important"; opting out of one is refused.
    pub can_opt_out: bool,
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
        Ok(Json(load_activity_tags(&self.db, user.get_id()).await?))
    }

    /// Opt out of notifications for an activity tag.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/activity-tags/:id/opt-out", method = "put")]
    async fn opt_out_activity_tag(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        let query = format!(
            "select {ACTIVITY_TAG_VISIBLE} as visible, tag.can_opt_out from activity_tags tag where tag.id = $2"
        );
        let (visible, can_opt_out) = sqlx::query_as::<_, (bool, bool)>(&query)
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
        if !can_opt_out {
            return Err(minilith_errors::MinilithEndpointError::bad_user_input(
                "tag can't be opted out of",
                "",
                "everyone gets this tag",
                "id",
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
        Ok(Json(load_news_tags(&self.db, user.get_id()).await?))
    }

    /// Opt out of push notifications for a news tag.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/news-tags/:id/opt-out", method = "put")]
    async fn opt_out_news_tag(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        let query = format!(
            "select {NEWS_TAG_VISIBLE} as visible, tag.can_opt_out from news_tags tag where tag.id = $2"
        );
        let (visible, can_opt_out) = sqlx::query_as::<_, (bool, bool)>(&query)
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
        if !can_opt_out {
            return Err(minilith_errors::MinilithEndpointError::bad_user_input(
                "tag can't be opted out of",
                "",
                "everyone gets this tag",
                "id",
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

/// The activity tags `user_id` can see, with whether they opted out of each.
pub(crate) async fn load_activity_tags(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<AppTag>> {
    // A runtime query, so plain column names: the `"name!: Type"` overrides only mean something
    // to the `query!` macros, and `row.get` looks columns up by their literal name.
    let query = format!(
        "select tag.id, tag.title, tag.creator_id, creator.path::text as creator_path,
            exists (
                select 1 from activity_opt_out_tags opt_out
                where opt_out.user_id = $1 and opt_out.tag_id = tag.id
            ) as opted_out,
            tag.can_opt_out
        from activity_tags tag
        inner join groups creator on creator.id = tag.creator_id
        where {ACTIVITY_TAG_VISIBLE}
        order by tag.title->>'en'"
    );
    sqlx::query(&query)
        .bind(user_id)
        .map(|row: sqlx::postgres::PgRow| {
            use sqlx::Row as _;
            AppTag {
                id: row.get("id"),
                title: row.get::<DIS, _>("title").0,
                creator_id: row.get("creator_id"),
                creator_path: row.get("creator_path"),
                opted_out: row.get("opted_out"),
                can_opt_out: row.get("can_opt_out"),
            }
        })
        .fetch_all(db)
        .await
}

/// The news tags `user_id` can see, with whether they opted out of each.
pub(crate) async fn load_news_tags(db: &PgPool, user_id: &str) -> sqlx::Result<Vec<AppTag>> {
    // A runtime query, so plain column names: the `"name!: Type"` overrides only mean something
    // to the `query!` macros, and `row.get` looks columns up by their literal name.
    let query = format!(
        "select tag.id, tag.title, tag.creator_id, creator.path::text as creator_path,
            exists (
                select 1 from news_push_opt_out_tags opt_out
                where opt_out.user_id = $1 and opt_out.tag_id = tag.id
            ) as opted_out,
            tag.can_opt_out
        from news_tags tag
        inner join groups creator on creator.id = tag.creator_id
        where {NEWS_TAG_VISIBLE}
        order by tag.title->>'en'"
    );
    sqlx::query(&query)
        .bind(user_id)
        .map(|row: sqlx::postgres::PgRow| {
            use sqlx::Row as _;
            AppTag {
                id: row.get("id"),
                title: row.get::<DIS, _>("title").0,
                creator_id: row.get("creator_id"),
                creator_path: row.get("creator_path"),
                opted_out: row.get("opted_out"),
                can_opt_out: row.get("can_opt_out"),
            }
        })
        .fetch_all(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(fixtures("notification_history", "notification_preferences"))]
    async fn lists_the_tags_a_user_can_see_with_their_opt_outs(db: sqlx::PgPool) {
        let db = sqlx_tracing::Pool::from(db);
        sqlx::query!(
            "update activity_tags set can_opt_out = false
            where id = '60000000-0000-0000-0000-000000000001'"
        )
        .execute(&db)
        .await
        .unwrap();

        let tags = load_activity_tags(&db, "important-off").await.unwrap();
        let summary: Vec<(String, bool, bool)> = tags
            .into_iter()
            .map(|tag| {
                (
                    tag.title.resolve_intl("en", "").to_owned(),
                    tag.opted_out,
                    tag.can_opt_out,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Important".to_owned(), true, false),
                ("Party".to_owned(), false, true),
            ]
        );

        let news = load_news_tags(&db, "eligible").await.unwrap();
        let opted_out: Vec<bool> = news.iter().map(|tag| tag.opted_out).collect();
        assert_eq!(opted_out, vec![true]);
    }
}
