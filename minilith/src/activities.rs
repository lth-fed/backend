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
#[derive(Object, Debug)]
pub struct Responsible {
    name: String,
    /// Should be tel: or mailto:
    contact: String,
}
#[derive(Object, Debug)]
pub struct Host {
    pub name: InternationalizedString,
    pub id: Uuid,
    pub path: String,
    pub logo_url: String,
}

#[derive(Object, Debug)]
pub struct Activity {
    pub id: Uuid,
    pub responsible: Responsible,
    /// The creator is the first in the `hosts` too.
    /// Used to find out which guild holds the event.
    pub sales_target_group_id: Uuid,
    pub title: InternationalizedString,
    pub description: InternationalizedString,
    pub location: PoemLocation,
    pub time_start: OffsetDateTime,
    pub time_end: OffsetDateTime,
    pub image_url: String,
    pub image_id: Uuid,
    /// Will always be true for users, but may vary for admins.
    pub is_hidden: bool,
    pub is_hidden_for_other_admins: bool,
    pub max_tickets: i32,
    pub hosts: Vec<Host>,
    /// If there are any tickets for this event.
    pub tickets_exist: bool,
    /// Where the activity's manually managed stock lives.
    pub stock_group_id: Option<Uuid>,
    /// Total number of times the activity has been opened.
    pub views: i64,
    /// The product that grants visibility; may also be a normal ticket.
    pub base_ticket: Uuid,
    /// Some if any tickets are purchasable.
    pub earliest_purchasable_ticket_release: Option<OffsetDateTime>,
}
#[derive(Object)]
struct BriefActivity {
    id: Uuid,
    /// Includes the creator and every additional host.
    host_ids: Vec<Uuid>,
    sales_target_group_name: InternationalizedString,
    sales_target_group_path: String,
    title: InternationalizedString,
    description: InternationalizedString,
    location: PoemLocation,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_url: String,
    /// If there are any tickets for this event.
    is_hidden: bool,
    earliest_purchasable_ticket_release: Option<OffsetDateTime>,
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

#[OpenApi(prefix_path = "/v0/activities")]
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
            host_ids: activity.host_ids,
            sales_target_group_path: activity.sales_target_group_path.to_string(),
            sales_target_group_name: activity.sales_target_group_name.0,
            title: activity.title.0,
            description: activity.description.0,
            location: activity.location.into(),
            time_start: activity.time_start,
            time_end: activity.time_end,
            image_url: activity.url,
            is_hidden: activity.is_hidden,
            earliest_purchasable_ticket_release: activity.earliest_ticket_release,
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
    pub async fn details(&self, user: User, id: Path<Uuid>) -> MinilithResult<Json<Activity>> {
        activity_details(self, user, id.0).await.map(Json)
    }
    /// The ticket kinds not purchasable due to membership check are not shown.
    ///
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
            r#"select kind.id,
            kind.name as "name!: DIS",
            (select base.price from product_base_prices base
                where base.product_id = kind.id) as "price!",
            kind.sales_start,
            kind.sales_stop,
            coalesce(kind.sales_stock, 0) as "available_tickets!"

            from products as kind
            inner join product_activity_assignments assignment
                on assignment.product_id = kind.id
            where assignment.activity_id = $1
            and kind.kind = 'ticket'
            and (
                (
                    exists (
                        select 1
                        from group_memberships
                        inner join groups member_group on member_group.id = group_memberships.group_id
                        inner join product_allowed_groups tk_ag on tk_ag.product_id = kind.id
                        inner join groups allowed_group on allowed_group.id = tk_ag.group_id
                            and allowed_group.path @> member_group.path

                        where group_memberships.user_id = $2
                        and (
                            not group_memberships.specific
                            or tk_ag.group_id = group_memberships.group_id
                        )
                    )
                )
                or exists (select 1 from product_buyers buyer
                    where buyer.activity_id = $1 and buyer.product_id = kind.id
                    and buyer.user_id = $2)
                -- admin
                or exists (select 1 from group_adminships admin
                    inner join activity_hosts host on admin.group_id = host.group_id
                    where admin.user_id = $2
                    and host.activity_id = $1
                )
            )
            "#,
            id.0,
            user.get_id()
        )
        .map(|kind| {
            ActivityTicketKind {
                id: kind.id,
                name: kind.name.0,
                price: kind.price.0,
                purchasing_available_start: kind.sales_start,
                purchasing_available_stop: kind.sales_stop,
                tickets_left: (kind.available_tickets < 10).then_some(kind.available_tickets),
            }
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(kinds))
    }
}

/// Loads an activity if the user may see it and records the view.
///
/// # Errors
///
/// - user might not be allowed to access this activity
/// - activity not found
#[allow(clippy::too_many_lines, reason = "linear activity mapping")]
pub(crate) async fn activity_details(
    ctx: &ContextWrapper,
    user: User,
    id: Uuid,
) -> MinilithResult<Activity> {
    let owns_ticket = sqlx::query_scalar!(
        r#"select exists (
                select 1
                from purchases
                where purchases.owner_id = $1
                and purchases.activity_id = $2
            ) as "exists!""#,
        user.get_id(),
        id,
    )
    .fetch_one(&ctx.db)
    .await?;
    if !owns_ticket {
        ctx.test_activity_access(user.get_id(), &id).await?;
    }
    let activity = sqlx::query!(
            r#"select activities.id,
                title as "title!: DIS",
                activities.description as "description!: DIS",
                location as "location!: Location", time_start, time_end,
                2147483647 as "max_tickets!",
                responsible_name,
                responsible_contact,
                images.url as "image_url",
                activities.image_id,
                (select target.group_id from sales_targets target where target.id = activities.sales_target_id) as "sales_target_group_id!",
                is_hidden,
                is_hidden_for_other_admins,
                stock_group as "stock_group_id?",
                base_ticket,
                coalesce((select sum(views.count) from activity_views views
                    where views.activity_id = activities.id), 0) as "views!",
                -- NOT SAME as activity-list.sql, because it filters for if it's not sold out
                -- either. This controls the tappableness of the button
                (select tk.sales_start
                    from products tk
                    inner join product_activity_assignments assignment on assignment.product_id = tk.id
                    where assignment.activity_id = $1
                    and tk.kind = 'ticket'
                    and (exists (
                        select 1 from product_allowed_groups g
                        inner join groups ag on ag.id = g.group_id
                        inner join groups ug on ag.path @> ug.path
                        inner join group_memberships m on m.group_id = ug.id
                        where g.product_id = tk.id and m.user_id = $2
                        and (not m.specific or g.group_id = m.group_id)
                    ) or exists (
                        select 1 from product_buyers b
                        where b.product_id = tk.id and b.activity_id = $1 and b.user_id = $2
                    ))
                    order by tk.sales_start
                    limit 1
                ) as "earliest_ticket_release?",
                (select exists (
                    select 1
                    from product_activity_assignments assignment
                    inner join products kind on kind.id = assignment.product_id
                    where assignment.activity_id = $1
                    and kind.kind = 'ticket'
                )) as "tickets_exist!"
            from activities
            inner join images on images.id = image_id
            where activities.id = $1;
            "#,
            id,
            user.get_id()
        )
        .fetch_optional(&ctx.db)
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
        id,
        activity.sales_target_group_id,
    )
    .fetch_all(&ctx.db)
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
        sales_target_group_id: activity.sales_target_group_id,
        views: activity.views,
        responsible: Responsible {
            name: activity.responsible_name,
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
        stock_group_id: activity.stock_group_id,
        base_ticket: activity.base_ticket,
        max_tickets: activity.max_tickets,
        hosts,
        tickets_exist: activity.tickets_exist,
        earliest_purchasable_ticket_release: activity.earliest_ticket_release,
    };

    sqlx::query!(
        "insert into activity_views (id, activity_id, user_id, count)
        values (uuidv4(), $1, $2, 1)
        on conflict (user_id) do update set
            activity_id = excluded.activity_id,
            count = activity_views.count + 1",
        id,
        user.get_id(),
    )
    .execute(&ctx.db)
    .await?;
    Ok(activity)
}
