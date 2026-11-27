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

#[derive(Debug, Clone, Object)]
pub struct AppTag {
    pub id: Uuid,
    pub title: IS,
    pub creator_id: Uuid,
    pub creator_path: String,
    /// Whether the caller has opted out of notifications for this tag.
    pub opted_out: bool,
}

/// Visible to a user if the tag's creator is an ancestor of (or equal to) one of their memberships.
const TAG_VISIBLE_TO_MEMBER: &str = "exists (
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
            where not tag.hidden and {TAG_VISIBLE_TO_MEMBER}
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
            "select not tag.hidden and {TAG_VISIBLE_TO_MEMBER} as \"visible!\" from activity_tags tag where tag.id = $2"
        );
        let visible = sqlx::query_scalar::<_, bool>(&query)
            .bind(user.get_id())
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .wrap_err_not_found()?;
        if !visible {
            return Err(MinilithEndpointError::unauthorized("unknown tag", ""));
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
            where not tag.hidden and {TAG_VISIBLE_TO_MEMBER}
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
            "select not tag.hidden and {TAG_VISIBLE_TO_MEMBER} as \"visible!\" from news_tags tag where tag.id = $2"
        );
        let visible = sqlx::query_scalar::<_, bool>(&query)
            .bind(user.get_id())
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            .wrap_err_not_found()?;
        if !visible {
            return Err(MinilithEndpointError::unauthorized("unknown tag", ""));
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

/// Admin tag management for a group's own tags (the Tags tab). Inherited tags are picked in the
/// news/activity/product editors, not edited here.
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

#[derive(Debug, Clone, Object)]
pub struct AdminTag {
    pub id: Uuid,
    pub creator_id: Uuid,
    pub creator_path: String,
    pub title: IS,
    pub hidden: bool,
}

#[derive(Debug, Object)]
pub struct PutAdminTag {
    /// The group that owns the tag; must be directly administered by the caller.
    pub creator_id: Uuid,
    pub title: IS,
}

/// Every tag kind shares `(id, owner, title, hidden)`, but `product_tags` names the owner
/// `group_id` and the title `name`. Tags are hidden instead of deleted, so existing assignments
/// and opt-outs stay valid.
#[derive(Clone, Copy)]
struct TagKind {
    table: &'static str,
    owner_column: &'static str,
    title_column: &'static str,
}

fn tag_kind(kind: &str) -> MinilithResult<TagKind> {
    Ok(match kind {
        "activity" => TagKind {
            table: "activity_tags",
            owner_column: "creator_id",
            title_column: "title",
        },
        "news" => TagKind {
            table: "news_tags",
            owner_column: "creator_id",
            title_column: "title",
        },
        "product" => TagKind {
            table: "product_tags",
            owner_column: "group_id",
            title_column: "name",
        },
        _ => {
            return Err(MinilithEndpointError::bad_user_input(
                "unknown tag kind",
                "",
                "kind must be activity, news or product",
                "kind",
            ));
        }
    })
}

#[OpenApi(prefix_path = "/v0/admin")]
impl AdminRouter {
    /// Lists the tags a group itself owns, including hidden ones, for the group's Tags tab.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `BAD_REQUEST` (unknown kind).
    #[oai(path = "/groups/:group_id/tags/:kind", method = "get")]
    async fn list_group_tags(
        &self,
        user: User,
        Path(group_id): Path<Uuid>,
        Path(kind): Path<String>,
    ) -> MinilithResult<Json<Vec<AdminTag>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id).await?;
        let kind = tag_kind(&kind)?;
        let rows = sqlx::query(&format!(
            r#"select tag.id, tag.{owner_column} as "creator_id",
                creator.path::text as "creator_path",
                tag.{title_column} as "title", tag.hidden
            from {table} tag
            inner join groups creator on creator.id = tag.{owner_column}
            where tag.{owner_column} = $1
            order by tag.{title_column}->>'en'"#,
            table = kind.table,
            owner_column = kind.owner_column,
            title_column = kind.title_column,
        ))
        .bind(group_id)
        .fetch_all(&self.db)
        .await?;
        let tags = rows
            .iter()
            .map(admin_tag_from_row)
            .collect::<MinilithResult<Vec<_>>>()?;
        Ok(Json(tags))
    }

    /// Creates or renames a tag owned by a group the caller directly administers.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `BAD_REQUEST` (unknown kind), `NOT_FOUND` (tag exists under another group).
    #[oai(path = "/tags/:kind/:id", method = "put")]
    async fn put_tag(
        &self,
        user: User,
        Path(kind): Path<String>,
        Path(id): Path<Uuid>,
        Json(body): Json<PutAdminTag>,
    ) -> MinilithResult<Json<AdminTag>> {
        check_direct_adminship(&self.db, user.get_id(), body.creator_id).await?;
        let kind = tag_kind(&kind)?;
        // Ownership can't change: a tag id owned by another group is refused by the WHERE.
        sqlx::query(&format!(
            "insert into {table} (id, {owner_column}, {title_column}) values ($1, $2, $3)
            on conflict (id) do update set {title_column} = excluded.{title_column}
            where {table}.{owner_column} = excluded.{owner_column}",
            table = kind.table,
            owner_column = kind.owner_column,
            title_column = kind.title_column,
        ))
        .bind(id)
        .bind(body.creator_id)
        .bind(body.title.to_json_value())
        .execute(&self.db)
        .await?;
        let row = sqlx::query(&format!(
            r#"select tag.id, tag.{owner_column} as "creator_id",
                creator.path::text as "creator_path",
                tag.{title_column} as "title", tag.hidden
            from {table} tag
            inner join groups creator on creator.id = tag.{owner_column}
            where tag.id = $1"#,
            table = kind.table,
            owner_column = kind.owner_column,
            title_column = kind.title_column,
        ))
        .bind(id)
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;
        Ok(Json(admin_tag_from_row(&row)?))
    }

    /// Hides a tag so it can no longer be picked. Tags are never removed, because existing
    /// assignments and opt-outs reference them.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `BAD_REQUEST` (unknown kind), `NOT_FOUND`.
    #[oai(path = "/tags/:kind/:id/hidden", method = "put")]
    async fn hide_tag(
        &self,
        user: User,
        Path(kind): Path<String>,
        Path(id): Path<Uuid>,
        Json(hidden): Json<bool>,
    ) -> MinilithResult<()> {
        let kind = tag_kind(&kind)?;
        let creator_id = sqlx::query_scalar(&format!(
            "select {owner_column} from {table} where id = $1",
            table = kind.table,
            owner_column = kind.owner_column,
        ))
        .bind(id)
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;
        check_direct_adminship(&self.db, user.get_id(), creator_id).await?;
        sqlx::query(&format!(
            "update {table} set hidden = $2 where id = $1",
            table = kind.table,
        ))
        .bind(id)
        .bind(hidden)
        .execute(&self.db)
        .await?;
        Ok(())
    }
}

fn admin_tag_from_row(row: &sqlx::postgres::PgRow) -> MinilithResult<AdminTag> {
    use sqlx::Row as _;
    Ok(AdminTag {
        id: row.try_get("id")?,
        creator_id: row.try_get("creator_id")?,
        creator_path: row.try_get("creator_path")?,
        title: serde_json::from_value(row.try_get::<serde_json::Value, _>("title")?)
            .map_err(|_| MinilithEndpointError::internal_error("l3: invalid tag title json", ""))?,
        hidden: row.try_get("hidden")?,
    })
}
