use std::ops::Deref;

use fed_auth_verifier::User;
use minilith_errors::MinilithEndpointError;
use poem_openapi::param::{Path, Query};
use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi};
use sqlx::postgres::types::PgPoint;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;

use crate::context::ContextWrapper;
use crate::{
    DbInternationalizedString as DIS, InternationalizedString, MinilithErrorOptionExt as _,
    MinilithResult,
};

#[derive(sqlx::Type, Debug)]
#[sqlx(type_name = "location")]
pub struct Location {
    name: Option<DIS>,
    directions: Option<DIS>,
    coordinate_wgs84: Option<PgPoint>,
    url: Option<String>,
}

#[derive(Debug, Object, Clone, Copy)]
pub struct Coordinates {
    pub north: f64,
    pub east: f64,
}
#[derive(Debug, Object)]
#[oai(rename = "Location")]
pub struct PoemLocation {
    pub name: Option<InternationalizedString>,
    pub directions: Option<InternationalizedString>,
    pub coordinate_wgs84: Option<Coordinates>,
    pub url: Option<String>,
}
impl From<Location> for PoemLocation {
    fn from(value: Location) -> Self {
        Self {
            name: value.name.map(|name| name.0),
            directions: value.directions.map(|dir| dir.0),
            coordinate_wgs84: value.coordinate_wgs84.map(|point| Coordinates {
                north: point.x,
                east: point.y,
            }),
            url: value.url,
        }
    }
}
#[derive(Object)]
struct Responsible {
    id: String,
    name: String,
    /// Should be tel: or mailto:
    contact: String,
}
#[derive(Object)]
struct Host {
    name: InternationalizedString,
    id: Uuid,
    path: String,
    logo_url: String,
}

#[derive(Object)]
struct Activity {
    id: Uuid,
    responsible: Responsible,
    /// The creator is the first in the `hosts` too.
    /// Used to find out which guild holds the event.
    creator_id: Uuid,
    creator_path: String,
    title: InternationalizedString,
    description: InternationalizedString,
    location: PoemLocation,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
    image_id: Uuid,
    /// Will always be true for users, but may vary for admins.
    is_hidden: bool,
    is_hidden_for_other_admins: bool,
    max_tickets: i32,
    hosts: Vec<Host>,
    /// If there are any tickets for this event.
    tickets_exist: bool,
}
#[derive(Object)]
struct BriefActivity {
    id: Uuid,
    creator_name: InternationalizedString,
    creator_path: String,
    title: InternationalizedString,
    description: InternationalizedString,
    location: PoemLocation,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
    /// If there are any tickets for this event.
    is_hidden: bool,
}

#[derive(Object)]
struct ActivityTicketKind {
    id: Uuid,
    name: InternationalizedString,
    price: i64,
    purchasing_available_start: OffsetDateTime,
    purchasing_available_stop: OffsetDateTime,
    /// Null if there's not a shortage of tickets.
    tickets_left: Option<i32>,
    membership_passing: bool,
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

#[OpenApi(prefix_path = "/activities")]
impl Router {
    /// `paging_start` & `paging_end` have to both be null or not null.
    /// They cannot be more than 50 days apart.
    /// That is mainly useful for admins, for normal users not including them results in all
    /// upcoming activities.
    ///
    /// # Errors
    ///
    /// DB, AUTH.
    #[oai(path = "/", method = "get")]
    async fn list(
        &self,
        user: User,
        paging_start: Query<Option<OffsetDateTime>>,
        paging_end: Query<Option<OffsetDateTime>>,
    ) -> MinilithResult<Json<Vec<BriefActivity>>> {
        match (paging_start.0, paging_end.0) {
            (Some(start), Some(end)) if end - start < time::Duration::ZERO => {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "paging interval negative",
                    "",
                ));
            }
            (Some(start), Some(end)) if end - start > time::Duration::DAY * 50 => {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "paging interval too long",
                    "",
                ));
            }
            (Some(_), Some(_)) | (None, None) => {}
            _ => {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "paging has to have both start and end",
                    "",
                ));
            }
        }
        let activities = sqlx::query_file!(
            "src/activity-list.sql",
            user.get_id(),
            paging_start.0,
            paging_end.0,
        )
        .map(|activity| BriefActivity {
            id: activity.id,
            creator_path: activity.creator_path.to_string(),
            creator_name: activity.creator_name.0,
            title: activity.title.0,
            description: activity.description.0,
            location: activity.location.into(),
            time_start: activity.time_start,
            time_end: activity.time_end,
            image_url: activity.url,
            is_hidden: activity.is_hidden,
        })
        .fetch_all(&self.context.db)
        .await?;

        Ok(Json(activities))
    }
    /// # Errors
    ///
    /// - user might not be allowed to access this activity
    /// - activity not found
    #[oai(path = "/:id", method = "get")]
    #[allow(
        clippy::too_many_lines,
        reason = "it's very easy to read"
    )]
    async fn details(&self, user: User, id: Path<Uuid>) -> MinilithResult<Json<Activity>> {
        let owns_ticket = sqlx::query_scalar!(
            r#"select exists (
                select 1
                from purchased_tickets
                inner join ticket_kinds
                    on ticket_kinds.id = purchased_tickets.ticket_kind_id
                where purchased_tickets.owner_id = $1
                and ticket_kinds.activity_id = $2
            ) as "exists!""#,
            user.get_id(),
            id.0,
        )
        .fetch_one(&self.db)
        .await?;
        if !owns_ticket {
            self.test_activity_access(user.get_id(), &id.0).await?;
        }
        let activity = sqlx::query!(
            r#"select activities.id,
                title as "title!: DIS",
                activities.description as "description!: DIS",
                location as "location!: Location", time_start, time_end,
                max_tickets,
                users.id as "responsible_id",
                responsible_contact,
                users.name as "responsible_name",
                users.nonce as "responsible_nonce",
                images.url as "image_url",
                activities.image_id,
                creator.id as creator_id,
                creator.path as creator_path,
                is_hidden,
                is_hidden_for_other_admins
            from activities
            inner join users on users.id = responsible_id
            inner join images on images.id = image_id
            inner join groups creator on creator.id = creator_id
            where activities.id = $1;
            "#,
            *id
        )
        .fetch_optional(&self.context.db)
        .await?
        .wrap_err_not_found()?;
        let hosts = sqlx::query!(
            r#"select hosts.id, path, name as "name!: DIS", url
            from activity_hosts
            inner join groups hosts on hosts.id = activity_hosts.group_id
            inner join images on hosts.logo_id = images.id
            where activity_id = $1
            order by hosts.id = $2 desc, hosts.path
            "#,
            *id,
            activity.creator_id,
        )
        .fetch_all(&self.context.db)
        .await?;
        let tickets_available = sqlx::query!(
            r#"select exists (
                select 1
                from ticket_kinds tk
                where tk.activity_id = $1
                and max_tickets > 0
            ) as value;
            "#,
            *id,
        )
        .fetch_one(&self.context.db)
        .await?;

        let hosts = hosts
            .into_iter()
            .map(|host| Host {
                name: host.name.0,
                id: host.id,
                path: host.path.to_string(),
                logo_url: host.url,
            })
            .collect();

        let activity = Activity {
            id: activity.id,
            creator_id: activity.creator_id,
            creator_path: activity.creator_path.to_string(),
            responsible: Responsible {
                id: activity.responsible_id,
                name: self
                    .decrypt_string(activity.responsible_name, &activity.responsible_nonce)
                    .wrap_err_encryption("responsible_name")?,
                contact: activity.responsible_contact,
            },
            title: activity.title.0,
            description: activity.description.0,
            location: activity.location.into(),
            time_start: activity.time_start,
            time_end: activity.time_end,
            image_url: activity.image_url,
            image_id: activity.image_id,
            is_hidden: activity.is_hidden,
            is_hidden_for_other_admins: activity.is_hidden_for_other_admins,
            max_tickets: activity.max_tickets,
            hosts,
            tickets_exist: tickets_available.value.unwrap_or(false),
        };

        Ok(Json(activity))
    }
    /// # Errors
    ///
    /// - user might not be allowed to access this activity
    #[oai(path = "/:id/ticket-kinds", method = "get")]
    async fn kinds(
        &self,
        user: User,
        id: Path<Uuid>,
    ) -> MinilithResult<Json<Vec<ActivityTicketKind>>> {
        self.test_activity_access(user.get_id(), &id.0).await?;
        let kinds = sqlx::query!(
            r#"
            select kind.id,
            kind.name as "name!: DIS",
            kind.price,
            kind.purchasing_available_start,
            kind.purchasing_available_stop,
            greatest(least(
                kind.max_tickets - kind.reserved_or_purchased_tickets,
                activities.max_tickets - (
                    select coalesce(sum(all_kinds.reserved_or_purchased_tickets), 0)::int
                    from ticket_kinds all_kinds
                    where all_kinds.activity_id = activities.id
                )
            ), 0)::int as "available_tickets!",
            exists (
                select 1
                from group_memberships
                inner join groups member_group on member_group.id = group_memberships.group_id
                inner join ticket_kind_allowed_groups tk_ag on tk_ag.ticket_kind_id = kind.id
                inner join groups allowed_group on allowed_group.id = tk_ag.group_id
                    and allowed_group.path @> member_group.path

                where group_memberships.user_id = $2
                and (
                    member_group.limit_membership_visibility = false
                    or tk_ag.group_id = group_memberships.group_id
                )
            ) as membership_passing

            from ticket_kinds as kind
            inner join activities on activities.id = kind.activity_id
            where kind.activity_id = $1
            and kind.max_tickets > 0
            "#,
            id.0,
            user.get_id()
        )
        .map(|kind| {
            // todo(release): check activity max too
            ActivityTicketKind {
                id: kind.id,
                name: kind.name.0,
                price: kind.price.0,
                purchasing_available_start: kind.purchasing_available_start,
                purchasing_available_stop: kind.purchasing_available_stop,
                tickets_left: (kind.available_tickets < 10).then_some(kind.available_tickets),
                membership_passing: kind.membership_passing.unwrap_or(false),
            }
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(kinds))
    }
}
