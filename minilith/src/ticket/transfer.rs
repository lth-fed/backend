use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use poem_openapi::Object;
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use super::{
    access::ensure_user_may_receive_transferred_ticket,
    ensure_affected_rows,
    flow::{reserve_user_purchase_flow, unlist_user_purchase_flow},
};
use crate::{ContextWrapper, MinilithEndpointError, MinilithResult};

#[derive(Object)]
pub(super) struct TransferRequest {
    purchased_ticket_id: Uuid,
    to_user: String,
}

pub(super) async fn transfer(
    ctx: &ContextWrapper,
    auth: User,
    body: TransferRequest,
) -> MinilithResult<()> {
    let to_user = if body.to_user.contains(':') {
        body.to_user.clone()
    } else {
        format!(
            "{}:{}",
            auth.get_id().split(':').next().unwrap_or(auth.get_id()),
            body.to_user
        )
    };
    let mut txn = ctx.db.begin().await?;
    if !sqlx::query_scalar!(
        "select exists (select 1 from users where id = $1) as \"exists!\"",
        to_user
    )
    .fetch_one(&mut txn.executor())
    .await?
    {
        return Err(MinilithEndpointError::bad_user_input(
            "to_user doesn't exist",
            "",
            "to_user doesn't exist",
            "to_user",
        ));
    }
    let ticket_kind = sqlx::query_scalar!(
        "select ticket_kind_id from purchased_tickets
            where id = $1 and owner_id = $2",
        body.purchased_ticket_id,
        auth.get_id()
    )
    .fetch_optional(&mut txn.executor())
    .await?
    .wrap_err_bad_frontend("you don't own this ticket")?;
    if auth.get_id() == to_user {
        return Ok(());
    }
    if sqlx::query_scalar!(
        "select exists (select 1 from purchased_ticket_validations where id = $1) as \"exists!\"",
        body.purchased_ticket_id
    )
    .fetch_one(&mut txn.executor())
    .await?
    {
        return Err(MinilithEndpointError::bad_user_input(
            "ticket has been used",
            "",
            "ticket has been used",
            "ticket_id",
        ));
    }

    reserve_user_purchase_flow(
        &mut txn,
        &[auth.get_id().to_owned(), to_user.clone()],
        ticket_kind,
        &[true, false],
    )
    .await?;

    // The ownership check is repeated while locking the ticket. It may have
    // changed while the flow locks above were being acquired.
    let row = sqlx::query!(
        "select allow_transfer_ticket_start, allow_transfer_ticket_stop
            from purchased_tickets
            inner join ticket_kinds kind on kind.id = purchased_tickets.ticket_kind_id
            where purchased_tickets.id = $1 and owner_id = $2
            for update of purchased_tickets",
        body.purchased_ticket_id,
        auth.get_id()
    )
    .fetch_optional(&mut txn.executor())
    .await?
    .wrap_err_bad_frontend("you don't own this ticket")?;

    let now = OffsetDateTime::now_utc();
    if row.allow_transfer_ticket_stop <= now || row.allow_transfer_ticket_start >= now {
        return Err(MinilithEndpointError::bad_frontend_code(
            "cannot transfer ticket at this time",
            "",
        ));
    }
    ensure_user_may_receive_transferred_ticket(&mut txn.executor(), &to_user, ticket_kind).await?;
    let affected = sqlx::query!(
        "update purchased_tickets set owner_id = $2 where id = $1",
        body.purchased_ticket_id,
        to_user
    )
    .execute(&mut txn.executor())
    .await?;
    ensure_affected_rows(
        affected.rows_affected(),
        1,
        "purchased ticket disappeared while transferring",
    )?;

    unlist_user_purchase_flow(&mut txn, auth.get_id()).await?;
    unlist_user_purchase_flow(&mut txn, &to_user).await?;

    txn.commit().await?;

    Ok(())
}
