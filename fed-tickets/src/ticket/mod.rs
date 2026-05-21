use std::collections::HashSet;
use std::ops::Deref;

use fed_auth_verifier::User;
use poem::{Error, http::StatusCode};
use poem_openapi::{Object, OpenApi, payload::Json};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::{Context, InternalServerError};

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

#[OpenApi(prefix_path = "/tickets")]
impl Router {
    #[oai(path = "/", method = "get")]
    async fn my_tickets(&self, user: User) {
        todo!()
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
