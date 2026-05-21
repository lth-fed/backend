use std::collections::{HashMap, HashSet};
use std::ops::Deref;

use fed_auth_verifier::User;
use poem::{Error, http::StatusCode};
use poem_openapi::{Object, OpenApi, payload::Json};
use sqlx::PgExecutor;
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    Context, DbInternationalizedString as DIS, InternalServerError, InternationalizedString as IS,
};

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

#[derive(Debug, Clone, Object)]
pub struct GetFreeTicketRequest {
    ticket_kind: Uuid,
    addons: Vec<Uuid>,
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

    #[oai(path = "/", method = "post")]
    async fn get_free_ticket(
        &self,
        user: User,
        req: Json<GetFreeTicketRequest>,
    ) -> poem::Result<Json<Uuid>> {
        let mut txn = self.db.begin().await.map_err(InternalServerError::db)?;

        validate_addons(&mut *txn, &req.addons, req.ticket_kind).await?;
        ensure_user_may_purchase_ticket(&mut *txn, &user, req.ticket_kind).await?;

        let ticket_id = sqlx::query_scalar!(
            "insert into purchased_tickets (ticket_kind_id, owner_id) values ($1, $2) returning id",
            req.ticket_kind,
            user.get_id(),
        )
        .fetch_one(&mut *txn)
        .await
        .map_err(|err| match err {
            sqlx::Error::Database(ref db_err) if let Some(constraint) = db_err.constraint() => {
                match constraint {
                    "max_one_ticket_per_person_per_activity" => Error::from_string(
                        "Max one ticket per person per activity",
                        StatusCode::CONFLICT,
                    ),
                    _unknown_constraint => InternalServerError::db(err).into(),
                }
            }
            other_err => InternalServerError::db(other_err).into(),
        })?;

        for addon in &req.addons {
            sqlx::query!(
                "insert into purchased_ticket_addons (addon_id, ticket_id) values ($1, $2)",
                addon,
                ticket_id
            )
            .execute(&mut *txn)
            .await
            .map_err(|err| InternalServerError::db(err))?;
        }

        // increment ticket_kinds.reserved_or_purchased_tickets and set
        // has_been_purchased to true
        sqlx::query!(
            "update ticket_kinds set reserved_or_purchased_tickets = reserved_or_purchased_tickets + 1, has_been_purchased = true where id = $1",
            req.ticket_kind
        )
        .execute(&mut *txn)
        .await
        .map_err(|err| InternalServerError::db(err))?;

        txn.commit()
            .await
            .map_err(|err| InternalServerError::db(err))?;

        Ok(Json(ticket_id))
    }
}

/// Ensure that the addons aren't duplicated and that they belong to the
/// specified `ticket_kind`.
async fn validate_addons(
    db: impl PgExecutor<'_>,
    addons: &[Uuid],
    ticket_kind: Uuid,
) -> poem::Result<()> {
    let mut seen = HashSet::new();
    for &addon in addons {
        if !seen.insert(addon) {
            return Err(Error::from_string(
                format!("Addon {addon} is duplicated"),
                StatusCode::BAD_REQUEST,
            ));
        }
    }

    let count = sqlx::query_scalar!(
        "select count(*) from ticket_addons where id = any($1) and ticket_kind_id = $2",
        addons,
        ticket_kind
    )
    .fetch_one(db)
    .await
    .map_err(InternalServerError::db)?;

    if count != Some(addons.len() as i64) {
        return Err(Error::from_string(
            "Invalid addons",
            StatusCode::BAD_REQUEST,
        ));
    }

    Ok(())
}

/// Ensure that the user may purchase a ticket of the specified `ticket_kind`
/// with regard to their group memberships.
///
/// If no allowed groups are configured for the ticket kind, anyone may
/// purchase. Otherwise the user must be a (transitive) member of at least one
/// allowed group — membership in a parent group covers all descendant groups.
///
/// # Errors
///
/// Returns 403 if the user is not allowed to purchase, or an internal error if
/// the database query fails.
async fn ensure_user_may_purchase_ticket(
    db: impl PgExecutor<'_>,
    user: &User,
    ticket_kind: Uuid,
) -> poem::Result<()> {
    let may_purchase = sqlx::query_scalar!(
        r#"select (
            not exists (select 1 from ticket_kind_allowed_groups where ticket_kind_id = $1)
            or
            exists (
                select 1
                from ticket_kind_allowed_groups tkag
                join groups allowed on allowed.id = tkag.group_id
                join group_memberships gm on gm.user_id = $2
                join groups member_group on member_group.id = gm.group_id
                where tkag.ticket_kind_id = $1
                and (
                    (gm.group_id = allowed.id)
                    or
                    (allowed.limit_membership_visibility = false and member_group.path @> allowed.path)
                )
            )
        ) as "may_purchase!""#,
        ticket_kind,
        user.get_id()
    )
    .fetch_one(db)
    .await
    .map_err(InternalServerError::db)?;

    if !may_purchase {
        return Err(Error::from_string(
            "not allowed to purchase this ticket kind",
            StatusCode::FORBIDDEN,
        ));
    }

    Ok(())
}
