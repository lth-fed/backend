use std::ops::Deref;

use fed_auth_verifier::User;
use poem::http::StatusCode;
use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi};
use sqlx::postgres::types::PgPoint;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;

use crate::context::Context;
use crate::{DbInternationalizedString as DIS, InternalServerError, InternationalizedString};

#[derive(sqlx::Type, Debug)]
#[sqlx(type_name = "location")]
struct Location {
    name: Option<DIS>,
    directions: Option<DIS>,
    coordinate_wgs84: Option<PgPoint>,
    url: Option<String>,
}

#[derive(Object)]
struct Coords {
    north: f64,
    east: f64,
}
#[derive(Object)]
#[oai(rename = "Location")]
struct PoemLocation {
    name: Option<InternationalizedString>,
    directions: Option<InternationalizedString>,
    coordinate_wgs84: Option<Coords>,
    url: Option<String>,
}
impl From<Location> for PoemLocation {
    fn from(value: Location) -> Self {
        Self {
            name: value.name.map(|name| name.0),
            directions: value.directions.map(|dir| dir.0),
            coordinate_wgs84: value.coordinate_wgs84.map(|point| Coords {
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
}
#[derive(Object)]
struct Host {
    name: InternationalizedString,
    logo_url: String,
}

#[derive(Object)]
struct Activity {
    id: Uuid,
    responsible: Responsible,
    /// The creator is the first in the `hosts` too.
    /// Used to find out which guild holds the event.
    creator_path: String,
    title: InternationalizedString,
    description: InternationalizedString,
    location: PoemLocation,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
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

#[OpenApi(prefix_path = "/activities")]
impl Router {
    #[oai(path = "/", method = "get")]
    async fn list(&self, user: User) -> poem::Result<Json<Vec<BriefActivity>>> {
        let mut activities = sqlx::query!(
            r#"select distinct on (a.id)
                a.id,
                creator.name as "creator_name!: DIS",
                creator.admin_path,
                a.title as "title!: DIS",
                a.description as "description!: DIS",
                a.location as "location: Location",
                a.time_start,
                a.time_end,
                img.url
            from group_members
            inner join groups member_group on member_group.admin_path = group_members.group_id
            -- get the ticket_kinds we're allowed to purchase
            inner join ticket_kind_allowed_groups tk_ag on tk_ag.group_id @> group_members.group_id
            inner join ticket_kinds kind on kind.id = tk_ag.ticket_kind_id
            -- get the activity
            inner join activities a on a.id = kind.activity_id

            -- extra data (not used for logic)
            inner join groups creator on creator.admin_path = a.creator_id
            inner join images img on img.id = a.image_id

            left join user_group_settings settings on
                settings.user_id = $1 and settings.group_id = a.creator_id

            where group_members.member_id = $1
            and (
                member_group.limit_membership_visibility = false
                or tk_ag.group_id = group_members.group_id
            )
            and time_end + '6 hours' > now()
            and (settings.visible is null or settings.visible = true)
            --order by time_start asc
            limit 100
            "#,
            user.get_id()
        )
        .map(|activity| BriefActivity {
            id: activity.id,
            creator_path: activity.admin_path.to_string(),
            creator_name: activity.creator_name.0,
            title: activity.title.0,
            description: activity.description.0,
            location: activity.location.into(),
            time_start: activity.time_start,
            time_end: activity.time_end,
            image_url: activity.url,
        })
        .fetch_all(&self.context.db)
        .await
        .map_err(InternalServerError::db)?;

        // can't sort in query because postgres doesn't allow separate columns for distinct on &
        // order by
        // this is kinda bad since we limit, so it there's more than 100 items some may be missed.
        activities.sort_by_key(|act| act.time_start);
        Ok(Json(activities))
    }
    #[oai(path = "/:id", method = "get")]
    async fn details(&self, user: User, id: Path<Uuid>) -> poem::Result<Json<Activity>> {
        // this clusterfuck is the same logic as above, which checks if this activity should be
        // visible
        sqlx::query!(
            r#"select exists (select 1
            from group_members
            inner join groups member_group on member_group.admin_path = group_members.group_id
            -- get the ticket_kinds we're allowed to purchase
            inner join ticket_kind_allowed_groups tk_ag on tk_ag.group_id @> group_members.group_id
            inner join ticket_kinds kind on kind.id = tk_ag.ticket_kind_id

            where group_members.member_id = $1
            and kind.activity_id = $2
            and (
                member_group.limit_membership_visibility = false
                or tk_ag.group_id = group_members.group_id
            ))
            "#,
            user.get_id(),
            id.0
        )
        .fetch_one(&self.context.db)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let activity = sqlx::query!(
            r#"select activities.id,
                title as "title!: DIS",
                activities.description as "description!: DIS",
                location as "location!: Location", time_start, time_end,
                max_tickets,
                users.id as "responsible_id",
                users.name as "responsible_name",
                users.nonce as "responsible_nonce",
                images.url as "image_url",
                creator.name as "creator_name!: DIS",
                creator_logo.url as "creator_logo_url",
                creator.admin_path
            from activities
            inner join users on users.id = responsible_id
            inner join images on images.id = image_id
            inner join groups creator on creator.admin_path = creator_id
            inner join images creator_logo on creator_logo.id = creator.logo_id
            where activities.id = $1;
            "#,
            *id
        )
        .fetch_one(&self.context.db)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
        let other_hosts = sqlx::query!(
            r#"select name as "name!: DIS", url
            from activity_hosts
            inner join groups hosts on hosts.admin_path = activity_hosts.group_id
            inner join images on hosts.logo_id = images.id
            where activity_id = $1
            "#,
            *id
        )
        .fetch_all(&self.context.db)
        .await
        .map_err(InternalServerError::db)?;
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
        .await
        .map_err(InternalServerError::db)?;

        let hosts = std::iter::once(Host {
            name: activity.creator_name.0,
            logo_url: activity.creator_logo_url,
        })
        .chain(other_hosts.into_iter().map(|other| Host {
            name: other.name.0,
            logo_url: other.url,
        }))
        .collect();

        let activity = Activity {
            id: activity.id,
            creator_path: activity.admin_path.to_string(),
            responsible: Responsible {
                id: activity.responsible_id,
                name: self
                    .decrypt_string(activity.responsible_name, &activity.responsible_nonce)
                    .ok_or(InternalServerError::encryption("activity.responsible_name"))?,
            },
            title: activity.title.0,
            description: activity.description.0,
            location: activity.location.into(),
            time_start: activity.time_start,
            time_end: activity.time_end,
            image_url: activity.image_url,
            hosts,
            tickets_exist: tickets_available.value.unwrap_or(false),
        };

        Ok(Json(activity))
    }
}
