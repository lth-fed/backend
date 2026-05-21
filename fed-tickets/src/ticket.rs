use std::collections::HashMap;
use std::ops::Deref;

use crate::{DbInternationalizedString as DIS, InternalServerError, InternationalizedString as IS};
use fed_auth_verifier::User;
use poem_openapi::{Object, OpenApi, payload::Json};
use sqlx::types::Uuid;
use sqlx::types::time::OffsetDateTime;

use crate::Context;

#[derive(Debug, Clone)]
pub struct Router {
    pub context: Context,
}

impl Deref for Router {
    type Target = Context;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[derive(Object)]
struct PurchasedAddon {
    idx: i32,
    multiple_alternatives: bool,
    has_text_field: bool,
    required: bool,
    selected_options: Vec<i32>,
    selected_text: String,
}

#[derive(Object)]
struct Ticket {
    id: Uuid,
    ticket_kind_id: Uuid,
    activity_id: Uuid,
    ticket_kind_name: IS,
    activity_title: IS,
    creator_name: IS,
    time_start: OffsetDateTime,
    time_end: OffsetDateTime,
    addons: Vec<PurchasedAddon>,
}

#[OpenApi(prefix_path = "/tickets")]
impl Router {
    #[oai(path = "/", method = "get")]
    async fn my_tickets(&self, user: User) -> poem::Result<Json<Vec<Ticket>>> {
        let id = user.get_id();

        let tickets = sqlx::query!(
            r#"select
                purchased_tickets.id as "id",
                purchased_tickets.ticket_kind_id as "ticket_kind_id",
                ticket_kinds.activity_id as "activity_id",
                ticket_kinds.name as "ticket_kind_name!: DIS",
                activities.title as "activity_title!: DIS",
                creator.name as "creator_name!: DIS",
                activities.time_start as "time_start",
                activities.time_end as "time_end"
            from purchased_tickets
            inner join ticket_kinds on ticket_kinds.id = purchased_tickets.ticket_kind_id
            inner join activities on activities.id = ticket_kinds.activity_id
            inner join groups creator on creator.id = activities.creator_id
            where purchased_tickets.owner_id = $1
            "#,
            id
        )
        .fetch_all(&self.context.db)
        .await
        .map_err(InternalServerError::db)?;

        let mut addons: HashMap<Uuid, Vec<PurchasedAddon>> = sqlx::query!(
            r#"select
                purchased_ticket_addons.ticket_id as "ticket_id",
                ticket_addons.idx as "idx",
                ticket_addons.multiple_alternatives as "multiple_alternatives",
                ticket_addons.has_text_field as "has_text_field",
                ticket_addons.required as "required",
                purchased_ticket_addons.selected_options as "selected_options",
                purchased_ticket_addons.selected_text as "selected_text"
            from purchased_tickets
            inner join purchased_ticket_addons on purchased_ticket_addons.ticket_id = purchased_tickets.id
            inner join ticket_addons on ticket_addons.id = purchased_ticket_addons.addon_id
            where purchased_tickets.owner_id = $1
            order by ticket_addons.idx
            "#,
            id
        )
        .map(|row| {
            (
                row.ticket_id,
                PurchasedAddon {
                    idx: row.idx,
                    multiple_alternatives: row.multiple_alternatives,
                    has_text_field: row.has_text_field,
                    required: row.required,
                    selected_options: row.selected_options,
                    selected_text: row.selected_text,
                },
            )
        })
        .fetch_all(&self.context.db)
        .await
        .map_err(InternalServerError::db)?
        .into_iter()
        .fold(HashMap::new(), |mut map, (ticket_id, addon)| {
            map.entry(ticket_id).or_default().push(addon);
            map
        });

        Ok(Json(
            tickets
                .into_iter()
                .map(|ticket| Ticket {
                    addons: addons.remove(&ticket.id).unwrap_or_default(),
                    id: ticket.id,
                    ticket_kind_id: ticket.ticket_kind_id,
                    activity_id: ticket.activity_id,
                    ticket_kind_name: ticket.ticket_kind_name.0,
                    activity_title: ticket.activity_title.0,
                    creator_name: ticket.creator_name.0,
                    time_start: ticket.time_start,
                    time_end: ticket.time_end,
                })
                .collect(),
        ))
    }
}
