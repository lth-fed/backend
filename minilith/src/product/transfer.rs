use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use poem_openapi::Object;
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use super::{
    access::ensure_user_may_receive_transferred_ticket,
    ensure_affected_rows,
    flow::{reserve_user_purchase_flow, unlist_user_purchase_flow},
    notifications::{TicketNotification, notify_ticket_users},
};
use crate::{ContextWrapper, MinilithEndpointError, MinilithResult};

#[derive(Object)]
pub struct TransferRequest {
    pub purchased_ticket_id: Uuid,
    pub to_user: String,
}

/// Whether a purchase may currently be transferred by its owner. Recipient group membership is
/// per-recipient and checked separately.
pub(super) fn is_transferable(
    owned_by_me: bool,
    used: bool,
    refunded: bool,
    resale_start: OffsetDateTime,
    resale_stop: OffsetDateTime,
    now: OffsetDateTime,
) -> bool {
    owned_by_me && !used && !refunded && resale_start <= now && now < resale_stop
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps transfer validation, lock order, and post-commit notification together"
)]
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
        "select product_id from purchases
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
        "select exists (select 1 from purchase_uses where purchase_id = $1) as \"exists!\"",
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
        r#"select resale_start, resale_stop,
            (purchases.refunded_id is not null) as "refunded!",
            exists (select 1 from purchase_uses
                where purchase_id = purchases.id) as "used!"
            from purchases
            inner join products kind on kind.id = purchases.product_id
            where purchases.id = $1 and owner_id = $2
            for update of purchases"#,
        body.purchased_ticket_id,
        auth.get_id()
    )
    .fetch_optional(&mut txn.executor())
    .await?
    .wrap_err_bad_frontend("you don't own this ticket")?;

    if !is_transferable(
        true,
        row.used,
        row.refunded,
        row.resale_start,
        row.resale_stop,
        OffsetDateTime::now_utc(),
    ) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "cannot transfer ticket at this time",
            "",
        ));
    }
    ensure_user_may_receive_transferred_ticket(&mut txn.executor(), &to_user, ticket_kind).await?;
    let affected = sqlx::query!(
        "update purchases set owner_id = $2 where id = $1",
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
    // The receiver never paid, so the row has no transaction: receipts and the "was this ticket
    // transferred here?" check (`transaction_id is null`) rely on it.
    sqlx::query!(
        "insert into purchase_transactions
        (purchase_id, seller_id, buyer_id, completed_at, transaction_id)
        values ($1, $2, $3, now(), null)",
        body.purchased_ticket_id,
        auth.get_id(),
        to_user,
    )
    .execute(&mut txn.executor())
    .await?;

    unlist_user_purchase_flow(&mut txn, auth.get_id()).await?;
    unlist_user_purchase_flow(&mut txn, &to_user).await?;

    txn.commit().await?;

    notify_ticket_users(
        ctx,
        ticket_kind,
        vec![to_user],
        TicketNotification::Transfer,
    );

    Ok(())
}
