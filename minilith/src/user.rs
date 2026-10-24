use std::ops::Deref;
use std::str::FromStr as _;

use fed_auth_verifier::{User, callbacks::AuthCallbackDataV1};
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _};
use poem_openapi::payload::Response;
use poem_openapi::{Enum, Object, OpenApi, payload::Json};
use sqlx::postgres::types::PgLTree;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;
use tracing::warn;

use crate::admin::ActivityNotificationKind;
use crate::context::ContextWrapper;
use crate::group::Path;
use crate::{
    DbInternationalizedString as DIS, InternationalizedString, MinilithErrorOptionExt as _,
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
#[derive(Object)]
struct MyGroup {
    id: Uuid,
    path: Path,
    name: InternationalizedString,
    description: InternationalizedString,
    logo_url: String,
}

#[derive(Object)]
struct Me {
    id: String,
    name: String,
    language: String,
    creation: OffsetDateTime,
    groups: Vec<MyGroup>,
    /// Groups this user directly administers.
    admin_group_ids: Vec<Uuid>,
}

#[derive(Clone, Copy, Debug, Enum, Eq, PartialEq)]
#[oai(rename_all = "lowercase")]
pub(crate) enum NotificationKind {
    Activity,
    Group,
}

#[derive(Debug, Object)]
pub(crate) struct Notification {
    id: Uuid,
    kind: NotificationKind,
    /// Activity news only: who it was sent to.
    audience: Option<ActivityNotificationKind>,
    activity_id: Option<Uuid>,
    /// Group announcements only.
    group_id: Option<Uuid>,
    /// The activity's image or the group's logo.
    image_url: Option<String>,
    sender: InternationalizedString,
    title: InternationalizedString,
    content: InternationalizedString,
    send_at: OffsetDateTime,
    read: bool,
    /// Dismissed notifications leave the inbox; an activity's updates still list them.
    dismissed: bool,
}

#[derive(Debug, Object)]
struct NotificationStateUpdate {
    /// Omitted to leave unchanged.
    read: Option<bool>,
    /// Omitted to leave unchanged.
    dismissed: Option<bool>,
}

#[derive(Debug, Object)]
struct ReadAllRequest {
    /// The newest `send_at` the user has seen. Everything sent at or before it becomes read;
    /// anything newer, which they haven't seen, stays unread.
    up_to: OffsetDateTime,
}

#[derive(Debug, Object)]
struct ActivityNotificationState {
    activity_id: Uuid,
    /// Whether the user gets the activity's general news: their own choice for it, otherwise what
    /// they want from its hosts, otherwise yes.
    follow: bool,
    /// Ticket-holder news reaches ticket holders whatever `follow` says.
    holds_ticket: bool,
}

/// The user's notifications, newest first, with their read state. With `activity` it is that
/// activity's notifications the user is eligible for — whether or not they chose to receive them —
/// otherwise the history: everything received in the last six months.
pub(crate) async fn load_notifications(
    db: &crate::PgPool,
    user_id: &str,
    activity: Option<Uuid>,
) -> Result<Vec<Notification>, sqlx::Error> {
    sqlx::query!(
        r#"select
            details.id as "id!",
            details.kind as "kind!",
            details.audience,
            details.activity_id,
            details.group_id,
            details.image_url,
            details.sender as "sender!: DIS",
            details.title as "title!: DIS",
            details.content as "content!: DIS",
            details.send_at as "send_at!",
            coalesce(
                state.read,
                details.send_at <= coalesce(watermark.read_up_to, users.creation)
            ) as "read!",
            coalesce(state.dismissed, false) as "dismissed!"
        from notification_recipients recipient
        inner join notification_details details on details.id = recipient.notification_id
        inner join users on users.id = recipient.user_id
        left join notification_read_watermarks watermark on watermark.user_id = recipient.user_id
        left join notification_read_states state
            on state.user_id = recipient.user_id and state.notification_id = details.id
        where recipient.user_id = $1
        and details.sent
        and (
            ($2::uuid is null
                and recipient.receives
                and details.send_at >= now() - '6 months'::interval)
            or details.activity_id = $2
        )
        order by details.send_at desc, details.id"#,
        user_id,
        activity,
    )
    .map(|row| Notification {
        id: row.id,
        kind: if row.kind == "group" {
            NotificationKind::Group
        } else {
            NotificationKind::Activity
        },
        audience: row
            .audience
            .as_deref()
            .and_then(ActivityNotificationKind::from_db),
        activity_id: row.activity_id,
        group_id: row.group_id,
        image_url: row.image_url,
        sender: row.sender.0,
        title: row.title.0,
        content: row.content.0,
        send_at: row.send_at,
        read: row.read,
        dismissed: row.dismissed,
    })
    .fetch_all(db)
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum, sqlx::Type)]
#[oai(rename_all = "lowercase")]
#[sqlx(type_name = "notification_level", rename_all = "lowercase")]
enum NotificationLevel {
    None,
    All,
}

#[derive(Debug, Object)]
struct GroupSetting {
    group_id: Uuid,
    visible: bool,
    notification_level: NotificationLevel,
}
#[derive(Debug, Object)]
struct ActivitySetting {
    activity_id: Uuid,
    follow: bool,
}

#[OpenApi(prefix_path = "/user")]
impl Router {
    /// Lists notifications from the last six months that the user is currently eligible to
    /// receive according to their memberships and notification settings.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/notifications", method = "get")]
    async fn notifications(&self, user: User) -> MinilithResult<Json<Vec<Notification>>> {
        let notifications = load_notifications(&self.db, user.get_id(), None).await?;
        Ok(Json(notifications))
    }

    /// Marks one notification read or unread, or dismisses it from the inbox (or restores it).
    ///
    /// # Errors
    ///
    /// AUTH, DB, 404 when the notification isn't one the user is eligible for.
    #[oai(path = "/notifications/:id/state", method = "put")]
    async fn update_notification_state(
        &self,
        user: User,
        id: poem_openapi::param::Path<Uuid>,
        Json(body): Json<NotificationStateUpdate>,
    ) -> MinilithResult<()> {
        let updated = sqlx::query!(
            "insert into notification_read_states (user_id, notification_id, read, dismissed)
            select $1, $2, $3, coalesce($4, false)
            where exists (
                select 1 from notification_recipients
                where user_id = $1 and notification_id = $2
            )
            on conflict (user_id, notification_id) do update set
                read = coalesce($3, notification_read_states.read),
                dismissed = coalesce($4, notification_read_states.dismissed)",
            user.get_id(),
            id.0,
            body.read,
            body.dismissed,
        )
        .execute(&self.db)
        .await?
        .rows_affected();
        if updated == 0 {
            return Err(MinilithEndpointError::not_found());
        }
        Ok(())
    }

    /// Marks everything sent up to `up_to` as read, including notifications individually marked
    /// unread. Dismissals are kept.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/notifications/read-all", method = "post")]
    async fn read_all_notifications(
        &self,
        user: User,
        Json(body): Json<ReadAllRequest>,
    ) -> MinilithResult<()> {
        let mut txn = self.db.begin().await?;
        sqlx::query!(
            "insert into notification_read_watermarks (user_id, read_up_to)
            values ($1, least($2, now()))
            on conflict (user_id) do update set
                read_up_to = greatest(notification_read_watermarks.read_up_to, excluded.read_up_to)",
            user.get_id(),
            body.up_to,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "update notification_read_states state set read = null
            from notifications
            where state.user_id = $1
            and notifications.id = state.notification_id
            and notifications.send_at <= $2",
            user.get_id(),
            body.up_to,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            "delete from notification_read_states
            where user_id = $1 and read is null and not dismissed",
            user.get_id(),
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(())
    }

    /// # Errors
    ///
    /// AUTH, DB, ENC.
    #[oai(path = "/", method = "get")]
    async fn me(&self, user: User) -> MinilithResult<Json<Me>> {
        let groups = sqlx::query!(
            r#"select
                groups.id, path as "path!: Path",
                name as "name!: DIS",
                description as "description!: DIS",
                url as logo_url from group_memberships 
            inner join groups on
                groups.id = group_memberships.group_id 
            inner join images logo on
                logo.id = groups.logo_id
            where user_id = $1
                and deleted = false"#,
            user.get_id()
        )
        .map(|group| MyGroup {
            id: group.id,
            path: group.path,
            name: group.name.0,
            description: group.description.0,
            logo_url: group.logo_url,
        })
        .fetch_all(&self.db)
        .await?;

        let user = sqlx::query!("select * from users where id = ($1)", user.get_id())
            .fetch_optional(&self.db)
            .await?
            .wrap_err_internal(
                "Your user object doesn't exist. Try logging out and then in again.",
            )?;

        let admin_group_ids = sqlx::query_scalar!(
            "select group_id from group_adminships where user_id = $1 order by group_id",
            user.id,
        )
        .fetch_all(&self.db)
        .await?;

        Ok(Json(Me {
            id: user.id,
            name: self
                .decrypt_string(user.name)
                .wrap_err_encryption("USER_ME_NAME")?,
            language: self
                .decrypt_string(user.language)
                .wrap_err_encryption("USER_ME_LANG")?,
            creation: user.creation,
            groups,
            admin_group_ids,
        }))
    }

    /// Stores the user's preferred language. It must be non-empty & consist of "0-9a-z-".
    #[oai(path = "/language", method = "put")]
    async fn update_language(
        &self,
        user: User,
        Json(language): Json<String>,
    ) -> MinilithResult<()> {
        if language.is_empty()
            || language.len() > 35
            || !language
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "invalid language: too long or empty or invalid characters",
                "language",
            ));
        }

        let encrypted_language = self.encrypt(&language);
        sqlx::query!(
            "update users set language = $1 where id = $2",
            encrypted_language,
            user.get_id(),
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Lists the user's explicit group filter settings. Groups without a row
    /// inherit from their nearest configured ancestor.
    #[oai(path = "/group-settings", method = "get")]
    async fn group_settings(&self, user: User) -> MinilithResult<Json<Vec<GroupSetting>>> {
        let settings = sqlx::query_as!(
            GroupSetting,
            r#"select
                group_id,
                visible,
                notification_level as "notification_level!: NotificationLevel"
            from user_group_settings
            where user_id = $1
            order by group_id"#,
            user.get_id(),
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(settings))
    }

    /// Creates or replaces an explicit group filter setting.
    #[oai(path = "/group-settings", method = "put")]
    #[allow(trivial_casts, reason = "sqlx custom enum parameter type override")]
    async fn update_group_setting(
        &self,
        user: User,
        Json(body): Json<GroupSetting>,
    ) -> MinilithResult<()> {
        sqlx::query!(
            "insert into user_group_settings
                (user_id, group_id, visible, notification_level)
            values ($1, $2, $3, $4)
            on conflict (group_id, user_id) do update set
                visible = excluded.visible,
                notification_level = excluded.notification_level",
            user.get_id(),
            body.group_id,
            body.visible,
            body.notification_level as NotificationLevel,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
    /// Removes an explicit group settings. Useful for resetting filters.
    #[oai(path = "/group-settings/:group_id", method = "delete")]
    async fn delete_group_setting(
        &self,
        user: User,
        group_id: poem_openapi::param::Path<Uuid>,
    ) -> MinilithResult<()> {
        sqlx::query!(
            "delete from user_group_settings
            where user_id = $1 and group_id = $2",
            user.get_id(),
            group_id.0,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
    /// Lists the user's explicit activity settings (follow-bell overrides) for current activities.
    #[oai(path = "/activity-settings", method = "get")]
    async fn activity_settings(&self, user: User) -> MinilithResult<Json<Vec<ActivitySetting>>> {
        let settings = sqlx::query_as!(
            ActivitySetting,
            r#"select
                activity_id,
                follow
            from activity_notification_overrides
            join activities on activities.id = activity_id
            where user_id = $1
            and activities.time_start > now() - '1 day'::interval
            order by activities.time_start, activities.id"#,
            user.get_id(),
        )
        .fetch_all(&self.db)
        .await?;
        Ok(Json(settings))
    }

    /// The user's effective notification setting for one activity — what its follow bell shows.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/activity-settings/:activity_id", method = "get")]
    async fn activity_setting(
        &self,
        user: User,
        activity_id: poem_openapi::param::Path<Uuid>,
    ) -> MinilithResult<Json<ActivityNotificationState>> {
        let state = sqlx::query!(
            r#"select
                follows_activity($1, $2) as "follow!",
                exists (
                    select 1 from purchased_tickets
                    inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
                    where ticket_kinds.activity_id = $2 and purchased_tickets.owner_id = $1
                ) as "holds_ticket!""#,
            user.get_id(),
            activity_id.0,
        )
        .fetch_one(&self.db)
        .await?;
        Ok(Json(ActivityNotificationState {
            activity_id: activity_id.0,
            follow: state.follow,
            holds_ticket: state.holds_ticket,
        }))
    }

    /// Creates or replaces the user's choice for one activity, overriding what its hosts' group
    /// notification levels would give.
    #[oai(path = "/activity-settings", method = "put")]
    #[allow(trivial_casts, reason = "sqlx custom enum parameter type override")]
    async fn update_activity_setting(
        &self,
        user: User,
        Json(body): Json<ActivitySetting>,
    ) -> MinilithResult<()> {
        sqlx::query!(
            "insert into activity_notification_overrides
                (user_id, activity_id, follow)
            values ($1, $2, $3)
            on conflict (user_id, activity_id) do update set
                follow = excluded.follow",
            user.get_id(),
            body.activity_id,
            body.follow,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }
    /// # Errors
    ///
    /// DB, PARAM.
    #[oai(path = "/auth-callback/v1", method = "post")]
    async fn auth_callback_v1(&self, cb_data: AuthCallbackDataV1) -> MinilithResult<Response<()>> {
        if let Some(name) = cb_data.full_name {
            let name = self.encrypt(&name);

            sqlx::query!(
                "insert into users (id, name, language)
                values ($1, $2, $3) on conflict (id) do update
                    set name = excluded.name",
                cb_data.sub,
                name,
                self.encrypt(cb_data.language.as_deref().unwrap_or(""))
            )
            .execute(&self.db)
            .await?;

            if let Some(guild) = cb_data.lth_guild {
                let guild_path = PgLTree::from_str(&format!("tlth.{}", guild.to_str()))
                    .wrap_err_internal("l1: failed to get ltree path from guild")?;
                // membership
                let gid = sqlx::query_scalar!(
                    "insert into group_memberships (user_id, group_id)
                    select $1 as user_id, id as group_id 
                    from groups where path = $2
                    -- so even if it exists we return it
                    on conflict (user_id, group_id) do update set group_id = excluded.group_id
                    returning group_id",
                    cb_data.sub,
                    guild_path
                )
                .fetch_one(&self.db)
                .await?;
                // visible + notifications from guild
                sqlx::query!(
                    "insert into user_group_settings (user_id, group_id,
                        visible, notification_level)
                    values ($1, $2, true, 'all'::notification_level)
                    on conflict do nothing",
                    cb_data.sub,
                    gid
                )
                .execute(&self.db)
                .await?;
                // visible - notifications from tlth
                sqlx::query!(
                    "insert into user_group_settings (user_id, group_id,
                        visible, notification_level)
                    select $1 as user_id, id as group_id, true as visible,
                    'none'::notification_level as notification_level
                    from groups where path = 'tlth'
                    on conflict do nothing",
                    cb_data.sub,
                )
                .execute(&self.db)
                .await?;
            } else {
                warn!(user_id = cb_data.sub, "User signed up without guild");
            }

            Ok(Response::new(()))
        } else {
            let has_user = sqlx::query_scalar!(
                "select exists (
                    select 1 from users where id = $1
                ) as \"exists!\"",
                cb_data.sub
            )
            .fetch_one(&self.db)
            .await?;
            if !has_user {
                return Ok(Response::new(()).status(poem::http::StatusCode::CREATED));
            }
            Ok(Response::new(()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    fn id(value: &str) -> Uuid {
        Uuid::parse_str(value).unwrap()
    }

    async fn notifications(
        db: &PgPool,
        user_id: &str,
        activity: Option<Uuid>,
    ) -> Vec<Notification> {
        let db = sqlx_tracing::Pool::from(db.clone());
        load_notifications(&db, user_id, activity).await.unwrap()
    }

    async fn history_ids(db: &PgPool, user_id: &str) -> Vec<Uuid> {
        notifications(db, user_id, None)
            .await
            .into_iter()
            .map(|notification| notification.id)
            .collect()
    }

    #[sqlx::test(fixtures("notification_history"))]
    async fn notification_history_uses_recipient_access_rules(db: PgPool) {
        assert_eq!(
            history_ids(&db, "eligible").await,
            vec![id("30000000-0000-0000-0000-000000000001")]
        );
        // `tlth.e` set to `none`: its announcements aren't received, so they aren't in the history.
        assert!(history_ids(&db, "muted").await.is_empty());
        assert_eq!(
            history_ids(&db, "private-member").await,
            vec![id("30000000-0000-0000-0000-000000000004")]
        );
        // Hiding a group from the feed (`visible = false`) and muting it don't stop ticket-holder
        // news, and neither does muting the activity itself.
        assert_eq!(
            history_ids(&db, "owner-muted").await,
            vec![id("30000000-0000-0000-0000-000000000005")]
        );
        assert_eq!(
            history_ids(&db, "owner-overridden").await,
            vec![id("30000000-0000-0000-0000-000000000005")]
        );

        let rules = sqlx::query!(
            r#"select user_id as "user_id!", receives as "receives!"
            from notification_recipients where notification_id = $1 order by user_id"#,
            id("30000000-0000-0000-0000-000000000006"),
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(
            rules
                .iter()
                .map(|rule| (&*rule.user_id, rule.receives))
                .collect::<Vec<_>>(),
            // Activity news follows the hosts' notification level, defaulting to on.
            vec![("eligible", true), ("muted", false), ("no-settings", true)]
        );

        sqlx::query!(
            "update activities set is_hidden = true
			where id = '50000000-0000-0000-0000-000000000001'"
        )
        .execute(&db)
        .await
        .unwrap();
        assert!(history_ids(&db, "owner-muted").await.is_empty());
        let hidden_rules = sqlx::query_scalar!(
            "select user_id from notification_recipients where notification_id = $1",
            id("30000000-0000-0000-0000-000000000006"),
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert!(hidden_rules.is_empty());
    }

    #[sqlx::test(fixtures("notification_history"))]
    async fn notifications_carry_what_they_are_about_and_their_read_state(db: PgPool) {
        let activity = id("50000000-0000-0000-0000-000000000001");
        let owner_news = notifications(&db, "owner-overridden", Some(activity)).await;
        assert_eq!(owner_news.len(), 1);
        let news = owner_news.first().unwrap();
        assert_eq!(news.kind, NotificationKind::Activity);
        assert_eq!(news.audience, Some(ActivityNotificationKind::TicketHolders));
        assert_eq!(news.activity_id, Some(activity));
        assert!(news.image_url.is_some());

        let history = notifications(&db, "eligible", None).await;
        let announcement = history.first().unwrap();
        assert_eq!(announcement.kind, NotificationKind::Group);
        assert_eq!(
            announcement.group_id,
            Some(id("20000000-0000-0000-0000-000000000002"))
        );
        // Sent before the account existed: read by default.
        assert!(announcement.read);

        sqlx::query!(
            "insert into notification_read_watermarks (user_id, read_up_to)
            values ('eligible', now() - interval '1 year')"
        )
        .execute(&db)
        .await
        .unwrap();
        assert!(
            !notifications(&db, "eligible", None)
                .await
                .first()
                .unwrap()
                .read
        );

        sqlx::query!(
            "insert into notification_read_states (user_id, notification_id, read, dismissed)
            values ('eligible', '30000000-0000-0000-0000-000000000001', true, true)"
        )
        .execute(&db)
        .await
        .unwrap();
        let history = notifications(&db, "eligible", None).await;
        let updated = history.first().unwrap();
        assert!(updated.read && updated.dismissed);
    }
}
