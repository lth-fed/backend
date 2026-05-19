use std::ops::Deref;

use poem::http::StatusCode;
use poem_openapi::param::Path;
use poem_openapi::payload::Json;
use poem_openapi::{Object, OpenApi};
use sqlx::postgres::types::PgPoint;
use sqlx::types::time::OffsetDateTime;
use sqlx::types::{JsonValue, Uuid};
use tracing::error;

use crate::context::Context;
use crate::{DbInternationalizedString, InternalServerError, InternationalizedString};

#[derive(sqlx::Type, Debug)]
#[sqlx(type_name = "location")]
struct Location {
    name: Option<DbInternationalizedString>,
    directions: Option<DbInternationalizedString>,
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
    name: JsonValue,
    logo_url: String,
}

#[derive(Object)]
struct Activity {
    id: Uuid,
    responsible: Responsible,
    title: JsonValue,
    description: JsonValue,
    location: PoemLocation,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
    hosts: Vec<Host>,
    /// If there are any tickets for this event.
    tickets_exist: bool,
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
    #[oai(path = "/:id", method = "get")]
    async fn details(&self, id: Path<Uuid>) -> poem::Result<Json<Activity>> {
        let activity = sqlx::query!(
            r#"select activities.id, title, activities.description,
                location as "location!: Location", time_start, time_end,
                max_tickets,
                users.id as "responsible_id",
                users.name as "responsible_name",
                users.nonce as "responsible_nonce",
                images.url as "image_url",
                creator.name as "creator_name",
                creator_logo.url as "creator_logo_url"
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
        .inspect_err(|err| error!("Failed to fetch activity details from db: {err}"))
        .map_err(|_| StatusCode::NOT_FOUND)?;
        let other_hosts = sqlx::query!(
            r#"select name, url
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
            ) as value;
            "#,
            *id,
        )
        .fetch_one(&self.context.db)
        .await
        .map_err(InternalServerError::db)?;

        let hosts = std::iter::once(Host {
            name: activity.creator_name,
            logo_url: activity.creator_logo_url,
        })
        .chain(other_hosts.into_iter().map(|other| Host {
            name: other.name,
            logo_url: other.url,
        }))
        .collect();

        let activity = Activity {
            id: activity.id,
            responsible: Responsible {
                id: activity.responsible_id,
                name: self
                    .decrypt_string(activity.responsible_name, &activity.responsible_nonce)
                    .ok_or(InternalServerError::encryption("activity.responsible_name"))?,
            },
            title: activity.title,
            description: activity.description,
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
