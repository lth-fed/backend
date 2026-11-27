//! Admin-facing news: group posts with tags and an optional questionnaire.
//!
//! A questionnaire is a `kind = 'questionnaire'` product whose add-ons live in a
//! `product_addon_sets` row that is not tied to an activity. The news post id is reused as both
//! the questionnaire product id and its add-on set id, which keeps creation idempotent.

use std::collections::{HashMap, HashSet};

use bin_common::Transaction;
use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi};
use sqlx::Row as _;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::admin_addons::save_addon;
use crate::group::admin::check_direct_adminship;
use crate::product::AvailableAddon;
use crate::{ContextWrapper, DbInternationalizedString as DIS, InternationalizedString as IS};
use crate::{MinilithEndpointError, MinilithResult};

async fn remove_questionnaire(txn: &mut Transaction<'_>, id: Uuid) -> MinilithResult<()> {
    sqlx::query!(
        "update news set questionnaire_product = null where id = $1",
        id
    )
    .execute(&mut txn.executor())
    .await?;
    // Keep answered questionnaires intact, including their allowed groups and answer history.
    let has_answers = sqlx::query_scalar!(
        r#"select exists (select 1 from purchases where product_id = $1) as "exists!""#,
        id
    )
    .fetch_one(&mut txn.executor())
    .await?;
    if !has_answers {
        sqlx::query!(
            "delete from product_allowed_groups where product_id = $1",
            id
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!("delete from products where id = $1", id)
            .execute(&mut txn.executor())
            .await?;
        sqlx::query!("delete from product_addon_sets where id = $1", id)
            .execute(&mut txn.executor())
            .await?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Router {
    pub context: ContextWrapper,
}

#[derive(Debug, Object)]
struct AdminNewsTag {
    id: Uuid,
    creator_id: Uuid,
    creator_path: String,
    title: IS,
}

#[derive(Debug, Object)]
struct AdminNewsSummary {
    id: Uuid,
    group_id: Uuid,
    group_path: String,
    group_name: IS,
    title: IS,
    content: IS,
    published_at: OffsetDateTime,
    sent: bool,
    n_sent: i64,
    n_opened: i64,
    n_dismissed: i64,
    has_questionnaire: bool,
    responses: i64,
}

#[derive(Debug, Object)]
struct AdminQuestionOption {
    id: Uuid,
    idx: i32,
    name: IS,
    responses: i64,
}

#[derive(Debug, Object)]
struct AdminQuestion {
    id: Uuid,
    name: IS,
    multiple_alternatives: bool,
    has_text_field: bool,
    required: bool,
    responses: i64,
    options: Vec<AdminQuestionOption>,
    text_responses: Vec<String>,
}

#[derive(Debug, Object)]
struct AdminQuestionnaire {
    product_id: Uuid,
    title: IS,
    responses: i64,
    questions: Vec<AdminQuestion>,
}

/// The push title starts with the group's name so recipients see who sent it.
fn title_with_sender(sender: &IS, title: &IS) -> IS {
    let prefixed = title
        .0
        .iter()
        .map(|(language, text)| {
            let name = sender.resolve_intl(language, "");
            let text = if name.is_empty() {
                text.clone()
            } else {
                format!("{name}: {text}")
            };
            (language.clone(), text)
        })
        .collect();
    IS(prefixed)
}

#[derive(Debug, Object)]
struct AdminNews {
    #[oai(flatten)]
    summary: AdminNewsSummary,
    tags: Vec<AdminNewsTag>,
    questionnaire: Option<AdminQuestionnaire>,
}

#[derive(Debug, Object)]
struct PutNews {
    group_id: Uuid,
    title: IS,
    content: IS,
    published_at: OffsetDateTime,
    tag_ids: Vec<Uuid>,
}

#[derive(Debug, Object)]
struct PutQuestionnaire {
    title: IS,
    /// Questions are add-ons; options use `name`, and their price is ignored (always free).
    questions: Vec<AvailableAddon>,
}

#[derive(Clone, Debug)]
pub struct AdminRouter {
    pub context: ContextWrapper,
}

impl std::ops::Deref for Router {
    type Target = ContextWrapper;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
impl std::ops::Deref for AdminRouter {
    type Target = ContextWrapper;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

fn dis(value: serde_json::Value) -> IS {
    serde_json::from_value(value).unwrap_or_else(|_| IS(HashMap::new()))
}

/// Makes sure the caller directly administers the group that owns the news post.
async fn check_news_access(
    db: &bin_common::PgPool,
    user_id: &str,
    news_id: Uuid,
) -> MinilithResult<Uuid> {
    let group_id = sqlx::query_scalar!("select target_group_id from news where id = $1", news_id)
        .fetch_optional(db)
        .await?
        .wrap_err_not_found()?;
    check_direct_adminship(db, user_id, group_id).await?;
    Ok(group_id)
}

/// Tags the group's admins may assign: tags created by an ancestor of (or the group itself).
async fn visible_tags(
    db: impl sqlx::PgExecutor<'_>,
    group_id: Uuid,
) -> MinilithResult<Vec<AdminNewsTag>> {
    let rows = sqlx::query(
        r#"select tag.id, tag.creator_id, creator.path::text as "creator_path",
            tag.title as "title"
        from news_tags tag
        inner join groups creator on creator.id = tag.creator_id
        inner join groups target on target.id = $1
        where creator.path @> target.path
        order by tag.title->>'en'"#,
    )
    .bind(group_id)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|row| {
            let title = row.try_get::<serde_json::Value, _>("title")?;
            Ok(AdminNewsTag {
                id: row.try_get("id")?,
                creator_id: row.try_get("creator_id")?,
                creator_path: row.try_get("creator_path")?,
                title: dis(title),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

fn news_summary(row: &sqlx::postgres::PgRow) -> MinilithResult<AdminNewsSummary> {
    Ok(AdminNewsSummary {
        id: row.try_get("id")?,
        group_id: row.try_get("group_id")?,
        group_path: row.try_get("group_path")?,
        group_name: dis(row.try_get("group_name")?),
        title: dis(row.try_get("title")?),
        content: dis(row.try_get("content")?),
        published_at: row.try_get("published_at")?,
        sent: row.try_get("sent")?,
        n_sent: row.try_get("n_sent")?,
        n_opened: row.try_get("n_opened")?,
        n_dismissed: row.try_get("n_dismissed")?,
        has_questionnaire: row.try_get("has_questionnaire")?,
        responses: row.try_get("responses")?,
    })
}

const SUMMARY_COLUMNS: &str = r#"
    news.id as "id",
    news.target_group_id as "group_id",
    groups.path::text as "group_path",
    groups.name as "group_name",
    news.title as "title",
    news.content as "content",
    news.published_at as "published_at",
    coalesce(notification.sent, false) as "sent",
    coalesce(stats.n_sent, 0) as "n_sent",
    coalesce(stats.n_opened, 0) as "n_opened",
    coalesce(stats.n_dismissed, 0) as "n_dismissed",
    news.questionnaire_product is not null as "has_questionnaire",
    coalesce((select count(*) from purchases
        where purchases.product_id = news.questionnaire_product), 0)::bigint as "responses"
"#;

const SUMMARY_JOINS: &str = "
    inner join groups on groups.id = news.target_group_id
    left join news_push_notifications link on link.news_id = news.id
    left join push_notifications notification on notification.id = link.notification_id
    left join notification_stats stats on stats.notification_id = notification.id
";

#[allow(
    clippy::too_many_lines,
    reason = "questionnaire + stats load in one pass"
)]
async fn load_questionnaire(
    db: &bin_common::PgPool,
    product_id: Uuid,
) -> MinilithResult<Option<AdminQuestionnaire>> {
    let Some(product) = sqlx::query("select name from products where id = $1")
        .bind(product_id)
        .fetch_optional(db)
        .await?
    else {
        return Ok(None);
    };

    let addons = sqlx::query(
        r#"select addon.id, addon.name as "name", addon.multiple_alternatives,
            addon.has_text_field, addon.required
        from product_addon_links link
        inner join product_addons addon on addon.id = link.addon_id
        where link.product_id = $1
        order by addon.idx"#,
    )
    .bind(product_id)
    .fetch_all(db)
    .await?;

    let option_rows = sqlx::query(
        r#"select option.id, option.product_addon_id, option.idx, option.name as "name"
        from product_addon_links link
        inner join product_addons addon on addon.id = link.addon_id
        inner join product_addon_options option on option.product_addon_id = addon.id
        where link.product_id = $1
        order by option.product_addon_id, option.idx"#,
    )
    .bind(product_id)
    .fetch_all(db)
    .await?;

    let mut options: HashMap<Uuid, Vec<(Uuid, i32, IS)>> = HashMap::new();
    for row in option_rows {
        let name: serde_json::Value = row.try_get("name")?;
        options
            .entry(row.try_get("product_addon_id")?)
            .or_default()
            .push((row.try_get("id")?, row.try_get("idx")?, dis(name)));
    }

    let answers = sqlx::query(
        "select addon.addon_id,
            purchased.selected_options,
            purchased.selected_text
        from purchases purchase
        inner join purchased_product_addons purchased on purchased.purchase_id = purchase.id
        where purchase.product_id = $1",
    )
    .bind(product_id)
    .fetch_all(db)
    .await?;

    let responses = sqlx::query_scalar::<_, i64>(
        "select count(*)::bigint from purchases where product_id = $1",
    )
    .bind(product_id)
    .fetch_one(db)
    .await?;

    let mut per_addon: HashMap<Uuid, Vec<(Vec<i32>, String)>> = HashMap::new();
    for row in answers {
        per_addon
            .entry(row.try_get("addon_id")?)
            .or_default()
            .push((
                row.try_get("selected_options")?,
                row.try_get("selected_text")?,
            ));
    }

    let questions = addons
        .into_iter()
        .map(|row| {
            let addon_id: Uuid = row.try_get("id")?;
            let name: serde_json::Value = row.try_get("name")?;
            let answers = per_addon.remove(&addon_id).unwrap_or_default();
            let count = i64::try_from(answers.len()).unwrap_or(i64::MAX);
            let option_rows = options.remove(&addon_id).unwrap_or_default();
            let option_counts: HashMap<i32, i64> = answers
                .iter()
                .flat_map(|(selected, _)| selected.iter())
                .fold(HashMap::new(), |mut counts, idx| {
                    *counts.entry(*idx).or_default() += 1;
                    counts
                });
            let options = option_rows
                .into_iter()
                .map(|(id, idx, name)| AdminQuestionOption {
                    id,
                    idx,
                    name,
                    responses: option_counts.get(&idx).copied().unwrap_or(0),
                })
                .collect();
            // A free-text answer may hold several answers, one per line.
            let text_responses = answers
                .into_iter()
                .flat_map(|(_, text)| {
                    text.split('\n')
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect();
            Ok(AdminQuestion {
                id: addon_id,
                name: dis(name),
                multiple_alternatives: row.try_get("multiple_alternatives")?,
                has_text_field: row.try_get("has_text_field")?,
                required: row.try_get("required")?,
                responses: count,
                options,
                text_responses,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let title: serde_json::Value = product.try_get("name")?;
    Ok(Some(AdminQuestionnaire {
        product_id,
        title: dis(title),
        responses,
        questions,
    }))
}

async fn load_news(db: &bin_common::PgPool, id: Uuid) -> MinilithResult<AdminNews> {
    let row = sqlx::query(&format!(
        "select {SUMMARY_COLUMNS} from news {SUMMARY_JOINS} where news.id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?
    .wrap_err_not_found()?;
    let summary = news_summary(&row)?;
    let tag_rows = sqlx::query(
        r#"select tag.id, tag.creator_id, creator.path::text as "creator_path", tag.title as "title"
        from news_assigned_tags assigned
        inner join news_tags tag on tag.id = assigned.tag_id
        inner join groups creator on creator.id = tag.creator_id
        where assigned.news_id = $1
        order by tag.title->>'en'"#,
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let tags = tag_rows
        .into_iter()
        .map(|row| {
            let title: serde_json::Value = row.try_get("title")?;
            Ok(AdminNewsTag {
                id: row.try_get("id")?,
                creator_id: row.try_get("creator_id")?,
                creator_path: row.try_get("creator_path")?,
                title: dis(title),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let questionnaire = match row.try_get::<Option<Uuid>, _>("questionnaire_product")? {
        Some(product_id) => load_questionnaire(db, product_id).await?,
        None => None,
    };
    Ok(AdminNews {
        summary,
        tags,
        questionnaire,
    })
}

#[OpenApi(prefix_path = "/v0/admin")]
impl AdminRouter {
    /// Lists news posts for a group the caller directly administers.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `NOT_FOUND`.
    #[oai(path = "/news", method = "get")]
    async fn list_news(
        &self,
        user: User,
        #[oai(name = "group_id")] group_id: poem_openapi::param::Query<Uuid>,
    ) -> MinilithResult<Json<Vec<AdminNewsSummary>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id.0).await?;
        let rows = sqlx::query(&format!(
            "select {SUMMARY_COLUMNS} from news {SUMMARY_JOINS}
            where news.target_group_id = $1
            order by news.published_at desc, news.id"
        ))
        .bind(group_id.0)
        .fetch_all(&self.db)
        .await?;
        Ok(Json(
            rows.iter()
                .map(news_summary)
                .collect::<MinilithResult<Vec<_>>>()?,
        ))
    }

    /// Loads one news post including its tags, questionnaire and answer stats.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `NOT_FOUND`.
    #[oai(path = "/news/:id", method = "get")]
    async fn get_news(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<Json<AdminNews>> {
        check_news_access(&self.db, user.get_id(), id).await?;
        Ok(Json(load_news(&self.db, id).await?))
    }

    /// Creates or replaces a news post. This also schedules its push notification.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `BAD_REQUEST`.
    #[oai(path = "/news/:id", method = "put")]
    #[allow(
        clippy::too_many_lines,
        reason = "keeps news + notification + tags atomic"
    )]
    async fn put_news(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<PutNews>,
    ) -> MinilithResult<Json<AdminNews>> {
        check_direct_adminship(&self.db, user.get_id(), body.group_id).await?;
        let existing_group =
            sqlx::query_scalar!("select target_group_id from news where id = $1", id)
                .fetch_optional(&self.db)
                .await?;
        if let Some(old_group) = existing_group {
            check_direct_adminship(&self.db, user.get_id(), old_group).await?;
        }

        let allowed: HashSet<Uuid> = visible_tags(&self.db, body.group_id)
            .await?
            .into_iter()
            .map(|tag| tag.id)
            .collect();
        if body.tag_ids.iter().any(|tag| !allowed.contains(tag)) {
            return Err(MinilithEndpointError::bad_frontend_code(
                "tag is not visible for this group",
                "",
            ));
        }

        let mut txn = self.db.begin().await?;
        // A sent notification is immutable; its attached news post is history.
        let existing = sqlx::query!(
            r#"select link.notification_id, notification.sent
            from news_push_notifications link
            inner join push_notifications notification on notification.id = link.notification_id
            where link.news_id = $1
            for update of notification"#,
            id
        )
        .fetch_optional(&mut txn.executor())
        .await?;
        if existing.as_ref().is_some_and(|row| row.sent) {
            return Err(MinilithEndpointError::bad_frontend_code(
                "cannot edit a sent news post",
                "create a new post instead",
            ));
        }
        let notification_id = existing
            .as_ref()
            .map_or_else(Uuid::new_v4, |row| row.notification_id);

        sqlx::query!(
            r#"insert into news (id, target_group_id, title, content, published_at)
            values ($1, $2, $3, $4, $5)
            on conflict (id) do update set
                target_group_id = excluded.target_group_id,
                title = excluded.title,
                content = excluded.content,
                published_at = excluded.published_at"#,
            id,
            body.group_id,
            body.title.to_json_value(),
            body.content.to_json_value(),
            body.published_at,
        )
        .execute(&mut txn.executor())
        .await?;

        let sender = sqlx::query_scalar!(
            r#"select name as "name!: DIS" from groups where id = $1"#,
            body.group_id
        )
        .fetch_one(&mut txn.executor())
        .await?;
        sqlx::query!(
            r#"insert into push_notifications (id, kind, sender, title, content, send_at)
            values ($1, 'news', $2, $3, $4, $5)
            on conflict (id) do update set
                sender = excluded.sender,
                title = excluded.title,
                content = excluded.content,
                send_at = excluded.send_at
            where push_notifications.sent = false"#,
            notification_id,
            sender.to_json_value(),
            title_with_sender(&sender, &body.title).to_json_value(),
            body.content.to_json_value(),
            body.published_at,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            r#"insert into news_push_notifications (id, news_id, notification_id)
            values ($1, $1, $2)
            on conflict (id) do update set notification_id = excluded.notification_id"#,
            id,
            notification_id,
        )
        .execute(&mut txn.executor())
        .await?;

        sqlx::query!("delete from news_assigned_tags where news_id = $1", id)
            .execute(&mut txn.executor())
            .await?;
        for tag in &body.tag_ids {
            sqlx::query!(
                "insert into news_assigned_tags (news_id, tag_id) values ($1, $2) on conflict do nothing",
                id,
                tag
            )
            .execute(&mut txn.executor())
            .await?;
        }
        txn.commit().await?;
        Ok(Json(load_news(&self.db, id).await?))
    }

    /// Deletes a news post that has not been sent yet.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `NOT_FOUND`, `BAD_REQUEST`.
    #[oai(path = "/news/:id", method = "delete")]
    async fn delete_news(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        check_news_access(&self.db, user.get_id(), id).await?;
        let mut txn = self.db.begin().await?;
        let sent = sqlx::query_scalar!(
            r#"select coalesce(notification.sent, false) as "sent!"
            from news
            left join news_push_notifications link on link.news_id = news.id
            left join push_notifications notification on notification.id = link.notification_id
            where news.id = $1"#,
            id
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .wrap_err_not_found()?;
        if sent {
            return Err(MinilithEndpointError::bad_frontend_code(
                "cannot delete a sent news post",
                "sent posts are retained as notification history",
            ));
        }
        sqlx::query!("delete from news_assigned_tags where news_id = $1", id)
            .execute(&mut txn.executor())
            .await?;
        let notification_id = sqlx::query_scalar!(
            "delete from news_push_notifications where news_id = $1 returning notification_id",
            id
        )
        .fetch_optional(&mut txn.executor())
        .await?;
        remove_questionnaire(&mut txn, id).await?;
        sqlx::query!("delete from news where id = $1", id)
            .execute(&mut txn.executor())
            .await?;
        if let Some(notification_id) = notification_id {
            sqlx::query!(
                "delete from push_notifications where id = $1",
                notification_id
            )
            .execute(&mut txn.executor())
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Lists news tags that may be assigned to posts of the given group.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`.
    #[oai(path = "/news-tags", method = "get")]
    async fn list_news_tags(
        &self,
        user: User,
        #[oai(name = "group_id")] group_id: poem_openapi::param::Query<Uuid>,
    ) -> MinilithResult<Json<Vec<AdminNewsTag>>> {
        check_direct_adminship(&self.db, user.get_id(), group_id.0).await?;
        Ok(Json(visible_tags(&self.db, group_id.0).await?))
    }

    /// Creates or replaces the questionnaire attached to a news post.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `BAD_REQUEST`.
    #[allow(
        clippy::too_many_lines,
        reason = "keeps product, addon set and links atomic"
    )]
    #[oai(path = "/news/:id/questionnaire", method = "put")]
    async fn put_questionnaire(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<PutQuestionnaire>,
    ) -> MinilithResult<Json<AdminNews>> {
        let group_id = check_news_access(&self.db, user.get_id(), id).await?;
        let mut txn = self.db.begin().await?;

        let sales_target = sqlx::query_scalar!(
            r#"select target.id from sales_targets target
            inner join groups owner on owner.id = target.group_id
            inner join groups news_group on news_group.id = $1
            where news_group.path @> owner.path
            order by target.group_id
            limit 1"#,
            group_id
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .ok_or_else(|| {
            MinilithEndpointError::bad_frontend_code(
                "the group has no payout account for questionnaires",
                "configure a sales target for the group or its guild first",
            )
        })?;
        // The post id doubles as the questionnaire product id and add-on set id.
        sqlx::query!(
            r#"insert into products (id, group_id, kind, name, description,
                sales_organization, stock_real, stock_unit, stock_price,
                sales_start, sales_stop, resale_start, resale_stop)
            values ($1, $2, 'questionnaire', $3, '{}'::jsonb, $4, 1, 'st', 0,
                now(), now() + interval '100 years', now(), now() + interval '100 years')
            on conflict (id) do update set name = excluded.name"#,
            id,
            group_id,
            body.title.to_json_value(),
            sales_target,
        )
        .execute(&mut txn.executor())
        .await?;
        // Free, so the base price books nothing.
        crate::admin_addons::save_base_price_addon(&mut txn, id, &body.title, 0, &[], &[]).await?;
        sqlx::query!(
            r#"insert into product_allowed_groups (product_id, group_id) values ($1, $2)
            on conflict do nothing"#,
            id,
            group_id,
        )
        .execute(&mut txn.executor())
        .await?;

        let previous: Vec<Uuid> =
            sqlx::query_scalar("select addon_id from product_addon_links where product_id = $1")
                .bind(id)
                .fetch_all(&mut txn.executor())
                .await?;
        let keep: HashSet<Uuid> = body
            .questions
            .iter()
            .map(|question| question.inner.id)
            .collect();
        for addon_id in previous.iter().filter(|addon| !keep.contains(addon)) {
            sqlx::query!(
                "delete from product_addon_links where product_id = $1 and addon_id = $2",
                id,
                addon_id
            )
            .execute(&mut txn.executor())
            .await?;
            sqlx::query!(
                r#"delete from product_addons where id = $1 and not exists (
                    select 1 from purchased_product_addons where addon_id = $1
                )"#,
                addon_id
            )
            .execute(&mut txn.executor())
            .await?;
        }
        for question in &body.questions {
            save_addon(&mut txn, id, question).await?;
            sqlx::query!(
                r#"insert into product_addon_links (product_id, addon_id) values ($1, $2)
                on conflict do nothing"#,
                id,
                question.inner.id,
            )
            .execute(&mut txn.executor())
            .await?;
        }

        sqlx::query!(
            "update news set questionnaire_product = $1 where id = $1",
            id
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(Json(load_news(&self.db, id).await?))
    }

    /// Removes the questionnaire from a post. Answers are kept when they already exist.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`.
    #[oai(path = "/news/:id/questionnaire", method = "delete")]
    async fn delete_questionnaire(&self, user: User, Path(id): Path<Uuid>) -> MinilithResult<()> {
        check_news_access(&self.db, user.get_id(), id).await?;
        let mut txn = self.db.begin().await?;
        remove_questionnaire(&mut txn, id).await?;
        txn.commit().await?;
        Ok(())
    }
}

#[OpenApi(prefix_path = "/v1")]
impl Router {
    /// News posts visible to the caller: posts from their member groups and all subgroups.
    ///
    /// # Errors
    ///
    /// `DB`.
    #[oai(path = "/news", method = "get")]
    async fn list_visible_news(&self, user: User) -> MinilithResult<Json<Vec<AdminNewsSummary>>> {
        let rows = sqlx::query(&format!(
            "select distinct {SUMMARY_COLUMNS} from news {SUMMARY_JOINS}
            inner join group_memberships membership on membership.user_id = $1
            inner join groups member_group on member_group.id = membership.group_id
            where groups.path @> member_group.path
            order by news.published_at desc, news.id"
        ))
        .bind(user.get_id())
        .fetch_all(&self.db)
        .await?;
        Ok(Json(
            rows.iter()
                .map(news_summary)
                .collect::<MinilithResult<Vec<_>>>()?,
        ))
    }

    /// One visible news post, including its questionnaire questions (not the answer stats).
    ///
    /// # Errors
    ///
    /// `DB`, `NOT_FOUND`.
    #[oai(path = "/news/:id", method = "get")]
    async fn get_visible_news(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<AdminNews>> {
        let visible = sqlx::query_scalar!(
            r#"select exists (
                select 1 from news
                inner join groups target on target.id = news.target_group_id
                inner join group_memberships membership on membership.user_id = $1
                inner join groups member_group on member_group.id = membership.group_id
                where news.id = $2 and target.path @> member_group.path
            ) as "visible!""#,
            user.get_id(),
            id
        )
        .fetch_one(&self.db)
        .await?;
        if !visible {
            return Err(MinilithEndpointError::not_found());
        }
        Ok(Json(load_news(&self.db, id).await?))
    }
}
