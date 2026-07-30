//! This contains all functionality specific to admins.
//!
//! Viewing data which normal users also view is handled by their respective functions instead.

use std::collections::HashMap;
use std::ops::Deref;

use fed_auth_verifier::User;
use minilith_errors::{MinilithEndpointError, MinilithErrorResultExt as _};
use poem_openapi::{Object, OpenApi, param::Path, payload::Json};
use s3::post_policy::PostPolicyExpiration;
use sqlx::postgres::types::PgMoney;
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;

use crate::activities::{Location, PoemLocation};
use crate::context::ContextWrapper;
use crate::group::{self};
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
struct ObjectUploadAllowanceRequest {
    /// Must not contain the `.`. I.e. `jpg`, `JPEG`, `png` is ok.
    extension: String,
}
/// See `Post File using FormData in Node.js` at
/// <https://www.npmjs.com/package/@aws-sdk/s3-presigned-post>
#[derive(Object)]
struct ObjectUploadAllowanceResponse {
    url: String,
    fields: HashMap<String, String>,
    dynamic_fields: HashMap<String, String>,

    /// the key you must upload to
    key: String,
    /// max size in bytes the object can be.
    max_size_bytes: u32,
}

#[derive(Debug, Object)]
struct Coordinates {
    north: f64,
    east: f64,
}

#[derive(Debug, Object)]
struct ActivityLocationRequest {
    name: Option<InternationalizedString>,
    directions: Option<InternationalizedString>,
    coordinate_wgs84: Option<Coordinates>,
    url: Option<String>,
}

#[derive(Debug, Object)]
struct PutActivity {
    responsible_id: String,
    /// Must use a `mailto:` or `tel:` URI.
    responsible_contact: String,
    creator_id: Uuid,
    title: InternationalizedString,
    description: InternationalizedString,
    location: ActivityLocationRequest,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_id: Uuid,
    is_hidden: bool,
    is_hidden_for_other_admins: bool,
    max_tickets: i32,
    host_ids: Vec<Uuid>,
}

#[derive(Debug, Object)]
struct AdminActivity {
    id: Uuid,
    responsible_id: String,
    responsible_contact: String,
    creator_id: Uuid,
    title: InternationalizedString,
    description: InternationalizedString,
    location: PoemLocation,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    image_id: Uuid,
    is_hidden: bool,
    is_hidden_for_other_admins: bool,
    max_tickets: i32,
    host_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Object)]
struct PutAddonOption {
    id: Uuid,
    name: InternationalizedString,
    price: i64,
    bookkeeping_prices: Vec<i64>,
    bookkeeping_price_categories: Vec<String>,
}

#[derive(Debug, Clone, Object)]
struct PutTicketAddon {
    id: Uuid,
    name: InternationalizedString,
    multiple_alternatives: bool,
    has_text_field: bool,
    required: bool,
    options: Vec<PutAddonOption>,
}

#[derive(Debug, Object)]
struct PutTicketKind {
    activity_id: Uuid,
    name: InternationalizedString,
    price: i64,
    purchasing_available_start: OffsetDateTime,
    purchasing_available_stop: OffsetDateTime,
    max_tickets: i32,
    min_tickets: i32,
    allow_transfer_ticket_start: OffsetDateTime,
    allow_transfer_ticket_stop: OffsetDateTime,
    allow_transfer_ticket_bypass_allowed_groups: bool,
    allowed_group_ids: Vec<Uuid>,
    addons: Vec<PutTicketAddon>,
}

#[derive(Debug, Object)]
struct AdminTicketKind {
    id: Uuid,
    activity_id: Uuid,
    name: InternationalizedString,
    price: i64,
    purchasing_available_start: OffsetDateTime,
    purchasing_available_stop: OffsetDateTime,
    max_tickets: i32,
    min_tickets: i32,
    reserved_or_purchased_tickets: i32,
    allow_transfer_ticket_start: OffsetDateTime,
    allow_transfer_ticket_stop: OffsetDateTime,
    allow_transfer_ticket_bypass_allowed_groups: bool,
    has_been_purchased: bool,
    has_been_released: bool,
    allowed_group_ids: Vec<Uuid>,
    addons: Vec<PutTicketAddon>,
}

#[derive(Debug, Object)]
struct PutTicketNotification {
    title: InternationalizedString,
    content: InternationalizedString,
    send_at: OffsetDateTime,
}

#[derive(Debug, Object)]
struct TicketNotification {
    kind: String,
    #[oai(flatten)]
    notification: PutTicketNotification,
}

#[derive(Debug, Clone, Object)]
struct AdminPurchasedAddon {
    addon_id: Uuid,
    selected_options: Vec<i32>,
    selected_text: String,
}

#[derive(Debug, Object)]
struct AdminPurchasedTicket {
    id: Uuid,
    ticket_kind_id: Uuid,
    purchaser_id: String,
    owner_id: String,
    transaction_id: Uuid,
    addons: Vec<AdminPurchasedAddon>,
}

impl Router {
    async fn check_any_direct_adminship(
        &self,
        user_id: &str,
        group_ids: &[Uuid],
    ) -> MinilithResult<()> {
        let allowed = sqlx::query_scalar!(
            r#"select exists (
                select 1 from group_adminships
                where user_id = $1 and group_id = any($2)
            ) as "exists!""#,
            user_id,
            group_ids,
        )
        .fetch_one(&self.db)
        .await?;
        if allowed {
            Ok(())
        } else {
            Err(MinilithEndpointError::bad_frontend_code(
                "must directly administer at least one activity host",
                "",
            ))
        }
    }

    async fn load_admin_activity(&self, id: Uuid) -> MinilithResult<AdminActivity> {
        let row = sqlx::query!(
            r#"select
                id,
                responsible_id,
                responsible_contact,
                creator_id,
                title as "title!: DIS",
                description as "description!: DIS",
                location as "location!: Location",
                time_start,
                time_end,
                image_id,
                is_hidden,
                is_hidden_for_other_admins,
                max_tickets
            from activities
            where id = $1"#,
            id,
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;
        let host_ids = sqlx::query_scalar!(
            r#"select group_id from activity_hosts
            where activity_id = $1
            order by group_id = $2 desc, group_id"#,
            id,
            row.creator_id,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(AdminActivity {
            id: row.id,
            responsible_id: row.responsible_id,
            responsible_contact: row.responsible_contact,
            creator_id: row.creator_id,
            title: row.title.0,
            description: row.description.0,
            location: row.location.into(),
            time_start: row.time_start,
            time_end: row.time_end,
            image_id: row.image_id,
            is_hidden: row.is_hidden,
            is_hidden_for_other_admins: row.is_hidden_for_other_admins,
            max_tickets: row.max_tickets,
            host_ids,
        })
    }

    async fn load_admin_ticket_kind(&self, id: Uuid) -> MinilithResult<AdminTicketKind> {
        let row = sqlx::query!(
            r#"select
                id, activity_id, name as "name!: DIS", price,
                purchasing_available_start, purchasing_available_stop,
                max_tickets, min_tickets, reserved_or_purchased_tickets,
                allow_transfer_ticket_start, allow_transfer_ticket_stop,
                allow_transfer_ticket_bypass_allowed_groups,
                (
                    has_been_purchased
                    or exists (
                        select 1 from purchased_tickets
                        where purchased_tickets.ticket_kind_id = ticket_kinds.id
                    )
                ) as "has_been_purchased!",
                has_been_released
            from ticket_kinds where id = $1"#,
            id,
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;
        let allowed_group_ids = sqlx::query_scalar!(
            r#"select group_id from ticket_kind_allowed_groups
            where ticket_kind_id = $1 order by group_id"#,
            id,
        )
        .fetch_all(&self.db)
        .await?;
        let options = sqlx::query!(
            r#"select
                ticket_addon_id,
                ticket_addon_options.id,
                ticket_addon_options.name as "name!: DIS",
                ticket_addon_options.price,
                bookkeeping_prices as "bookkeeping_prices!: Vec<i64>",
                bookkeeping_price_categories
            from ticket_addon_options
            inner join ticket_addons on ticket_addons.id = ticket_addon_id
            where ticket_addons.ticket_kind_id = $1
            order by ticket_addon_id, ticket_addon_options.idx"#,
            id,
        )
        .map(|option| {
            (
                option.ticket_addon_id,
                PutAddonOption {
                    id: option.id,
                    name: option.name.0,
                    price: option.price.0,
                    bookkeeping_prices: option.bookkeeping_prices,
                    bookkeeping_price_categories: option.bookkeeping_price_categories,
                },
            )
        })
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .fold(
            HashMap::<Uuid, Vec<PutAddonOption>>::new(),
            |mut map, item| {
                map.entry(item.0).or_default().push(item.1);
                map
            },
        );
        let addons = sqlx::query!(
            r#"select
                id, name as "name!: DIS",
                multiple_alternatives, has_text_field, required
            from ticket_addons
            where ticket_kind_id = $1
            order by idx"#,
            id,
        )
        .map(|addon| PutTicketAddon {
            id: addon.id,
            name: addon.name.0,
            multiple_alternatives: addon.multiple_alternatives,
            has_text_field: addon.has_text_field,
            required: addon.required,
            options: options.get(&addon.id).cloned().unwrap_or_default(),
        })
        .fetch_all(&self.db)
        .await?;
        Ok(AdminTicketKind {
            id: row.id,
            activity_id: row.activity_id,
            name: row.name.0,
            price: row.price.0,
            purchasing_available_start: row.purchasing_available_start,
            purchasing_available_stop: row.purchasing_available_stop,
            max_tickets: row.max_tickets,
            min_tickets: row.min_tickets,
            reserved_or_purchased_tickets: row.reserved_or_purchased_tickets,
            allow_transfer_ticket_start: row.allow_transfer_ticket_start,
            allow_transfer_ticket_stop: row.allow_transfer_ticket_stop,
            allow_transfer_ticket_bypass_allowed_groups: row
                .allow_transfer_ticket_bypass_allowed_groups,
            has_been_purchased: row.has_been_purchased,
            has_been_released: row.has_been_released,
            allowed_group_ids,
            addons,
        })
    }
}

#[OpenApi(prefix_path = "/admin")]
impl Router {
    /// Creates or fully replaces an activity. Existing activities require a
    /// direct adminship in any current host; new activities require one in any
    /// submitted host.
    #[oai(path = "/activities/:id", method = "put")]
    async fn put_activity(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<PutActivity>,
    ) -> MinilithResult<Json<AdminActivity>> {
        if !body.responsible_id.starts_with("email:") {
            return Err(MinilithEndpointError::bad_frontend_code(
                "the responsible account must use email authentication",
                "",
            ));
        }
        if !(body.responsible_contact.starts_with("mailto:")
            || body.responsible_contact.starts_with("tel:"))
        {
            return Err(MinilithEndpointError::bad_frontend_code(
                "responsible_contact must start with `mailto:` or `tel:`",
                "",
            ));
        }

        let mut host_ids = body.host_ids;
        if !host_ids.contains(&body.creator_id) {
            host_ids.push(body.creator_id);
        }
        host_ids.sort_unstable();
        host_ids.dedup();

        let exists = sqlx::query_scalar!(
            r#"select exists (select 1 from activities where id = $1) as "exists!""#,
            id
        )
        .fetch_one(&self.db)
        .await?;
        if exists {
            group::admin::check_activity_adminship(&self.db, user.get_id(), id).await?;
        } else {
            self.check_any_direct_adminship(user.get_id(), &host_ids)
                .await?;
        }

        let mut txn = self.db.begin().await?;
        if exists {
            sqlx::query_scalar!("select id from activities where id = $1 for update", id,)
                .fetch_one(&mut txn.executor())
                .await?;
        }
        let currently_reserved = sqlx::query_scalar!(
            r#"select coalesce(sum(reserved_or_purchased_tickets), 0)::int
            from ticket_kinds where activity_id = $1"#,
            id,
        )
        .fetch_one(&mut txn.executor())
        .await?
        .unwrap_or(0);
        if body.max_tickets < currently_reserved {
            return Err(MinilithEndpointError::bad_frontend_code(
                "activity max_tickets is below its reserved or purchased ticket count",
                "",
            ));
        }

        let name = body
            .location
            .name
            .map(InternationalizedString::to_json_value);
        let directions = body
            .location
            .directions
            .map(InternationalizedString::to_json_value);
        let (north, east) = body
            .location
            .coordinate_wgs84
            .map_or((None, None), |point| (Some(point.north), Some(point.east)));

        sqlx::query!(
            r#"insert into activities (
                id, responsible_id, responsible_contact, creator_id,
                title, description, location, time_start, time_end, image_id,
                is_hidden, is_hidden_for_other_admins, max_tickets
            )
            values (
                $1, $2, $3, $4, $5, $6,
                row(
                    $7::jsonb,
                    $8::jsonb,
                    case when $9::float8 is null or $10::float8 is null
                        then null else point($9, $10) end,
                    $11
                )::location,
                $12, $13, $14, $15, $16, $17
            )
            on conflict (id) do update set
                responsible_id = excluded.responsible_id,
                responsible_contact = excluded.responsible_contact,
                creator_id = excluded.creator_id,
                title = excluded.title,
                description = excluded.description,
                location = excluded.location,
                time_start = excluded.time_start,
                time_end = excluded.time_end,
                image_id = excluded.image_id,
                is_hidden = excluded.is_hidden,
                is_hidden_for_other_admins = excluded.is_hidden_for_other_admins,
                max_tickets = excluded.max_tickets"#,
            id,
            body.responsible_id,
            body.responsible_contact,
            body.creator_id,
            body.title.to_json_value(),
            body.description.to_json_value(),
            name,
            directions,
            north,
            east,
            body.location.url,
            body.time_start,
            body.time_end,
            body.image_id,
            body.is_hidden,
            body.is_hidden_for_other_admins,
            body.max_tickets,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!("delete from activity_hosts where activity_id = $1", id)
            .execute(&mut txn.executor())
            .await?;
        sqlx::query!(
            r#"insert into activity_hosts (activity_id, group_id)
            select $1, group_id from unnest($2::uuid[]) as host(group_id)"#,
            id,
            &host_ids,
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;

        Ok(Json(self.load_admin_activity(id).await?))
    }

    /// Lists purchased tickets and addon selections for an activity.
    #[oai(path = "/activities/:id/purchased-tickets", method = "get")]
    async fn purchased_tickets(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<AdminPurchasedTicket>>> {
        group::admin::check_activity_adminship(&self.db, user.get_id(), id).await?;
        let addons = sqlx::query!(
            r#"select
                purchased_ticket_addons.ticket_id,
                purchased_ticket_addons.addon_id,
                purchased_ticket_addons.selected_options,
                purchased_ticket_addons.selected_text
            from purchased_tickets
            inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
            inner join purchased_ticket_addons
                on purchased_ticket_addons.ticket_id = purchased_tickets.id
            where ticket_kinds.activity_id = $1"#,
            id,
        )
        .map(|row| {
            (
                row.ticket_id,
                AdminPurchasedAddon {
                    addon_id: row.addon_id,
                    selected_options: row.selected_options,
                    selected_text: row.selected_text,
                },
            )
        })
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .fold(
            HashMap::<Uuid, Vec<AdminPurchasedAddon>>::new(),
            |mut map, item| {
                map.entry(item.0).or_default().push(item.1);
                map
            },
        );
        let tickets = sqlx::query!(
            r#"select
                purchased_tickets.id,
                purchased_tickets.ticket_kind_id,
                purchased_tickets.purchaser_id,
                purchased_tickets.owner_id,
                purchased_tickets.transaction_id
            from purchased_tickets
            inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
            where ticket_kinds.activity_id = $1
            order by purchased_tickets.id"#,
            id,
        )
        .map(|row| AdminPurchasedTicket {
            id: row.id,
            ticket_kind_id: row.ticket_kind_id,
            purchaser_id: row.purchaser_id,
            owner_id: row.owner_id,
            transaction_id: row.transaction_id,
            addons: addons.get(&row.id).cloned().unwrap_or_default(),
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(tickets))
    }

    /// Creates or fully replaces an unpurchased ticket kind, including its
    /// allowlist, addons, and options. After the first purchase, only the
    /// purchasing window and option bookkeeping may change.
    #[oai(path = "/ticket-kinds/:id", method = "put")]
    async fn put_ticket_kind(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(mut body): Json<PutTicketKind>,
    ) -> MinilithResult<Json<AdminTicketKind>> {
        let existing_id = sqlx::query_scalar!("select id from ticket_kinds where id = $1", id,)
            .fetch_optional(&self.db)
            .await?;
        if existing_id.is_some() {
            group::admin::check_ticket_kind_adminship(&self.db, user.get_id(), id).await?;
        }
        group::admin::check_activity_adminship(&self.db, user.get_id(), body.activity_id).await?;

        body.allowed_group_ids.sort_unstable();
        body.allowed_group_ids.dedup();
        let existing = if existing_id.is_some() {
            Some(self.load_admin_ticket_kind(id).await?)
        } else {
            None
        };

        if let Some(existing) = existing.as_ref().filter(|ticket| ticket.has_been_purchased) {
            let immutable_top_level_matches = existing.activity_id == body.activity_id
                && existing.name == body.name
                && existing.price == body.price
                && existing.max_tickets == body.max_tickets
                && existing.min_tickets == body.min_tickets
                && existing.allow_transfer_ticket_start == body.allow_transfer_ticket_start
                && existing.allow_transfer_ticket_stop == body.allow_transfer_ticket_stop
                && existing.allow_transfer_ticket_bypass_allowed_groups
                    == body.allow_transfer_ticket_bypass_allowed_groups
                && existing.allowed_group_ids == body.allowed_group_ids
                && existing.addons.len() == body.addons.len();
            let immutable_addons_match =
                immutable_top_level_matches
                    && existing.addons.iter().zip(&body.addons).all(|(old, new)| {
                        old.id == new.id
                            && old.name == new.name
                            && old.multiple_alternatives == new.multiple_alternatives
                            && old.has_text_field == new.has_text_field
                            && old.required == new.required
                            && old.options.len() == new.options.len()
                            && old.options.iter().zip(&new.options).all(
                                |(old_option, new_option)| {
                                    old_option.id == new_option.id
                                        && old_option.name == new_option.name
                                        && old_option.price == new_option.price
                                },
                            )
                    });
            if !immutable_addons_match {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "a purchased ticket kind's structure and pricing are immutable",
                    "only purchasing availability and option bookkeeping may change",
                ));
            }

            let mut txn = self.db.begin().await?;
            sqlx::query!(
                r#"update ticket_kinds set
                    purchasing_available_start = $2,
                    purchasing_available_stop = $3
                where id = $1"#,
                id,
                body.purchasing_available_start,
                body.purchasing_available_stop,
            )
            .execute(&mut txn.executor())
            .await?;
            for addon in &body.addons {
                for option in &addon.options {
                    let prices: Vec<PgMoney> = option
                        .bookkeeping_prices
                        .iter()
                        .copied()
                        .map(PgMoney)
                        .collect();
                    sqlx::query!(
                        r#"update ticket_addon_options set
                            bookkeeping_prices = $2,
                            bookkeeping_price_categories = $3
                        where id = $1 and ticket_addon_id = $4"#,
                        option.id,
                        &prices,
                        &option.bookkeeping_price_categories,
                        addon.id,
                    )
                    .execute(&mut txn.executor())
                    .await?;
                }
            }
            txn.commit().await?;
            return Ok(Json(self.load_admin_ticket_kind(id).await?));
        }

        let already_reserved = existing
            .as_ref()
            .map_or(0, |ticket| ticket.reserved_or_purchased_tickets);
        if body.max_tickets < already_reserved {
            return Err(MinilithEndpointError::bad_frontend_code(
                "ticket kind max_tickets is below its reservation count",
                "",
            ));
        }
        let mut txn = self.db.begin().await?;
        let mut activity_ids = vec![body.activity_id];
        if let Some(existing) = &existing {
            activity_ids.push(existing.activity_id);
        }
        activity_ids.sort_unstable();
        activity_ids.dedup();
        sqlx::query_scalar!(
            r#"select id from activities
            where id = any($1)
            order by id
            for update"#,
            &activity_ids,
        )
        .fetch_all(&mut txn.executor())
        .await?;
        if existing_id.is_some() {
            let purchased_now = sqlx::query_scalar!(
                r#"select (
                    has_been_purchased
                    or exists (
                        select 1 from purchased_tickets
                        where purchased_tickets.ticket_kind_id = ticket_kinds.id
                    )
                ) as "purchased!"
                from ticket_kinds
                where id = $1
                for update"#,
                id,
            )
            .fetch_one(&mut txn.executor())
            .await?;
            if purchased_now {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "the ticket kind was purchased while it was being edited",
                    "retry to update only purchasing availability and bookkeeping",
                ));
            }
        }
        let activity_capacity_valid = sqlx::query_scalar!(
            r#"select (
                coalesce(sum(ticket_kinds.reserved_or_purchased_tickets)
                    filter (where ticket_kinds.id <> $2), 0)
                + $3
            ) <= activities.max_tickets
            from activities
            left join ticket_kinds on ticket_kinds.activity_id = activities.id
            where activities.id = $1
            group by activities.max_tickets"#,
            body.activity_id,
            id,
            i64::from(already_reserved),
        )
        .fetch_one(&mut txn.executor())
        .await?
        .unwrap_or(false);
        if !activity_capacity_valid {
            return Err(MinilithEndpointError::bad_frontend_code(
                "activity max_tickets is below its reservation count",
                "",
            ));
        }

        sqlx::query!(
            r#"insert into ticket_kinds (
                id, activity_id, name, price,
                purchasing_available_start, purchasing_available_stop,
                max_tickets, min_tickets, reserved_or_purchased_tickets,
                allow_transfer_ticket_start, allow_transfer_ticket_stop,
                allow_transfer_ticket_bypass_allowed_groups,
                has_been_purchased, has_been_released
            ) values (
                $1, $2, $3, $4, $5, $6, $7, $8, 0, $9, $10, $11, false, false
            )
            on conflict (id) do update set
                activity_id = excluded.activity_id,
                name = excluded.name,
                price = excluded.price,
                purchasing_available_start = excluded.purchasing_available_start,
                purchasing_available_stop = excluded.purchasing_available_stop,
                max_tickets = excluded.max_tickets,
                min_tickets = excluded.min_tickets,
                allow_transfer_ticket_start = excluded.allow_transfer_ticket_start,
                allow_transfer_ticket_stop = excluded.allow_transfer_ticket_stop,
                allow_transfer_ticket_bypass_allowed_groups =
                    excluded.allow_transfer_ticket_bypass_allowed_groups"#,
            id,
            body.activity_id,
            body.name.to_json_value(),
            PgMoney(body.price),
            body.purchasing_available_start,
            body.purchasing_available_stop,
            body.max_tickets,
            body.min_tickets,
            body.allow_transfer_ticket_start,
            body.allow_transfer_ticket_stop,
            body.allow_transfer_ticket_bypass_allowed_groups,
        )
        .execute(&mut txn.executor())
        .await?;

        sqlx::query!(
            "delete from ticket_kind_allowed_groups where ticket_kind_id = $1",
            id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            r#"insert into ticket_kind_allowed_groups (ticket_kind_id, group_id)
            select $1, group_id from unnest($2::uuid[]) allowed(group_id)"#,
            id,
            &body.allowed_group_ids,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            r#"delete from ticket_addon_options
            where ticket_addon_id in (
                select id from ticket_addons where ticket_kind_id = $1
            )"#,
            id,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!("delete from ticket_addons where ticket_kind_id = $1", id)
            .execute(&mut txn.executor())
            .await?;

        for (addon_idx, addon) in body.addons.iter().enumerate() {
            #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
            let addon_idx = addon_idx as i32;
            sqlx::query!(
                r#"insert into ticket_addons (
                    id, ticket_kind_id, idx, name,
                    multiple_alternatives, has_text_field, required
                ) values ($1, $2, $3, $4, $5, $6, $7)"#,
                addon.id,
                id,
                addon_idx,
                addon.name.clone().to_json_value(),
                addon.multiple_alternatives,
                addon.has_text_field,
                addon.required,
            )
            .execute(&mut txn.executor())
            .await?;
            for (option_idx, option) in addon.options.iter().enumerate() {
                #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
                let option_idx = option_idx as i32;
                let prices: Vec<PgMoney> = option
                    .bookkeeping_prices
                    .iter()
                    .copied()
                    .map(PgMoney)
                    .collect();
                sqlx::query!(
                    r#"insert into ticket_addon_options (
                        id, ticket_addon_id, idx, name, price,
                        bookkeeping_prices, bookkeeping_price_categories
                    ) values ($1, $2, $3, $4, $5, $6, $7)"#,
                    option.id,
                    addon.id,
                    option_idx,
                    option.name.clone().to_json_value(),
                    PgMoney(option.price),
                    &prices,
                    &option.bookkeeping_price_categories,
                )
                .execute(&mut txn.executor())
                .await?;
            }
        }
        txn.commit().await?;
        Ok(Json(self.load_admin_ticket_kind(id).await?))
    }

    /// Creates or replaces a named notification for a ticket kind.
    #[oai(
        path = "/ticket-kinds/:ticket_kind_id/notifications/:kind",
        method = "put"
    )]
    async fn put_ticket_notification(
        &self,
        user: User,
        Path(ticket_kind_id): Path<Uuid>,
        Path(kind): Path<String>,
        Json(body): Json<PutTicketNotification>,
    ) -> MinilithResult<Json<TicketNotification>> {
        group::admin::check_ticket_kind_adminship(&self.db, user.get_id(), ticket_kind_id).await?;
        let mut txn = self.db.begin().await?;
        sqlx::query_scalar!(
            "select id from ticket_kinds where id = $1 for update",
            ticket_kind_id,
        )
        .fetch_one(&mut txn.executor())
        .await?;
        let notification_id = sqlx::query_scalar!(
            r#"select notification_id from ticket_kind_notifications
            where ticket_kind_id = $1 and id = $2"#,
            ticket_kind_id,
            kind,
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .unwrap_or_else(Uuid::new_v4);
        sqlx::query!(
            r#"insert into notifications (id, title, content, send_at)
            values ($1, $2, $3, $4)
            on conflict (id) do update set
                title = excluded.title,
                content = excluded.content,
                send_at = excluded.send_at"#,
            notification_id,
            body.title.clone().to_json_value(),
            body.content.clone().to_json_value(),
            body.send_at,
        )
        .execute(&mut txn.executor())
        .await?;
        sqlx::query!(
            r#"insert into ticket_kind_notifications
                (id, ticket_kind_id, notification_id)
            values ($1, $2, $3)
            on conflict (id, ticket_kind_id) do update set
                notification_id = excluded.notification_id"#,
            kind,
            ticket_kind_id,
            notification_id,
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(Json(TicketNotification {
            kind,
            notification: body,
        }))
    }

    /// Gets a named ticket-kind notification.
    #[oai(
        path = "/ticket-kinds/:ticket_kind_id/notifications/:kind",
        method = "get"
    )]
    async fn get_ticket_notification(
        &self,
        user: User,
        Path(ticket_kind_id): Path<Uuid>,
        Path(kind): Path<String>,
    ) -> MinilithResult<Json<TicketNotification>> {
        group::admin::check_ticket_kind_adminship(&self.db, user.get_id(), ticket_kind_id).await?;
        let row = sqlx::query!(
            r#"select
                notifications.title as "title!: DIS",
                notifications.content as "content!: DIS",
                notifications.send_at
            from ticket_kind_notifications
            inner join notifications
                on notifications.id = ticket_kind_notifications.notification_id
            where ticket_kind_notifications.ticket_kind_id = $1
            and ticket_kind_notifications.id = $2"#,
            ticket_kind_id,
            kind,
        )
        .fetch_optional(&self.db)
        .await?
        .wrap_err_not_found()?;
        Ok(Json(TicketNotification {
            kind,
            notification: PutTicketNotification {
                title: row.title.0,
                content: row.content.0,
                send_at: row.send_at,
            },
        }))
    }

    /// # Extension
    ///
    /// Only the following extensions are allowed (case doesn't matter):
    ///
    /// - jpg
    /// - jpeg
    /// - webp
    /// - png
    /// - avif
    ///
    /// Notably, no GIF support.
    ///
    /// # Errors
    ///
    /// - You must be admin for some group.
    /// - Extension has to be valid
    /// - internal errors
    #[oai(path = "/upload-image", method = "post")]
    async fn upload_image(
        &self,
        user: User,
        Json(mut body): Json<ObjectUploadAllowanceRequest>,
    ) -> MinilithResult<Json<ObjectUploadAllowanceResponse>> {
        // only admins can upload
        group::admin::check_has_any_adminship(&self.db, user.get_id()).await?;

        body.extension.make_ascii_lowercase();
        if !matches!(
            body.extension.as_str(),
            "jpg" | "jpeg" | "webp" | "png" | "avif"
        ) {
            return Err(MinilithEndpointError::bad_frontend_code(
                "invalid extension",
                "",
            ));
        }

        let uuid = Uuid::new_v4();
        let max_size_bytes = 1024u32 * 1024 * 4;
        let key = format!("{uuid}.{}", body.extension);
        let policy = s3::post_policy::PostPolicy::new(PostPolicyExpiration::ExpiresIn(60 * 5))
            .condition(
                s3::PostPolicyField::Key,
                s3::PostPolicyValue::Exact(key.as_str().into()),
            )
            .wrap_err_internal("s3: bad key condition")?
            .condition(
                s3::PostPolicyField::ContentLengthRange,
                s3::PostPolicyValue::Range(0, max_size_bytes),
            )
            .wrap_err_internal("s3: bad content length condition")?
            .sign(self.image_bucket().clone().into())
            .await
            .wrap_err_internal("s3: failed to sign")?;

        Ok(Json(ObjectUploadAllowanceResponse {
            url: policy.url,
            fields: policy.fields,
            dynamic_fields: policy.dynamic_fields,
            key,
            max_size_bytes,
        }))
    }
}
