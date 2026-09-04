use bin_common::Transaction;
use fed_auth_verifier::{User, callbacks::TransactionState};
use minilith_errors::{
    AlertLevel, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, alert,
};
use tracing::error;
use uuid::Uuid;

use super::{
    allocation::give_reservations_in_new_transaction,
    flow::{
        PurchaseFlow, attach_operation_lock_to_flow, detach_operation_lock_to_flow,
        invalidate_wait_for_user_purchase_flow_on_transaction_id, lock_user_purchase_flow,
        unlist_user_purchase_flow, wait_for_user_purchase_flow,
        wait_for_user_purchase_flow_on_transaction_id,
    },
    models::{BoughtAddon, BuyTicketRequest, BuyTicketResponse, PurchaseProvider},
};
use crate::{
    ContextWrapper, DbInternationalizedString as DIS, MinilithEndpointError, MinilithResult,
    transactions,
};

#[allow(
    clippy::too_many_lines,
    reason = "keeps the purchase transaction and external-payment sequence visible"
)]
pub(super) async fn begin(
    ctx: &ContextWrapper,
    user: User,
    mut body: BuyTicketRequest,
) -> MinilithResult<BuyTicketResponse> {
    if body.provider == PurchaseProvider::Stripe && body.stripe_success_url.is_none() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "stripe_success_url has to be non-null when provider is stripe!",
            "",
        ));
    }

    // ========
    // Get UUID (first, so we don't hog a DB connection)
    // ========
    let transaction_id: Uuid = ctx
        .transactions_post("/v0/init")
        .send()
        .await
        .wrap_err_internal("init transport failed")?
        .error_for_status()
        .wrap_err_internal("init status")?
        .json()
        .await
        .wrap_err_internal("l1: init bad type")?;

    let mut txn = ctx.db.begin().await?;
    let chosen_options = validate_addons(&mut txn, &mut body.addons, body.ticket_kind).await?;

    // this is here so nobody else tries to mess with our reservation while we are assigning it
    // a transaction_ìd
    let flow =
        lock_user_purchase_flow(&mut txn, user.get_id(), body.ticket_kind.into(), None).await?;
    if !matches!(*flow, PurchaseFlow::Reservation) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "you don't have a reservation right now",
            "",
        ));
    }
    let reservation = sqlx::query!(
        "select ticket_reservations.id, ticket_kind_id, timeout, transaction_id,
                kind.name as \"ticket_kind_name!: DIS\",
                activities.title as \"activity_title!: DIS\",
                kind.price
            from ticket_reservations
            inner join ticket_kinds kind on (kind.id = ticket_kind_id)
            inner join activities on (activities.id = kind.activity_id)
            where user_id = $1
            and activities.time_end > now()",
        user.get_id()
    )
    .fetch_optional(&mut txn.executor())
    .await?
    .wrap_err_not_found()?;
    if let Some(txn_id) = reservation.transaction_id {
        let lock_id = Uuid::new_v4();
        attach_operation_lock_to_flow(&mut txn, user.get_id(), lock_id).await?;
        // drop transaction before we start the cancel
        txn.commit().await?;

        let resp = ctx
            .transactions_post(format!("/v0/{txn_id}/cancel"))
            .send()
            .await;
        txn = ctx.db.begin().await?;
        wait_for_user_purchase_flow(
            &mut txn,
            user.get_id(),
            body.ticket_kind.into(),
            Some(lock_id),
        )
        .await?
        .wrap_err_bad_frontend("cancel took too long, flow gone, purchase complete")?;
        detach_operation_lock_to_flow(&mut txn, user.get_id()).await?;

        match resp {
            Ok(resp) => match resp.status() {
                reqwest::StatusCode::NOT_FOUND => {
                    // it's already cancelled
                }
                reqwest::StatusCode::FORBIDDEN => {
                    txn.commit().await?;
                    return Err(MinilithEndpointError::bad_user_input(
                        "tried to cancel when disallowed",
                        txn_id,
                        "cannot cancel your current transaction at this point",
                        "cancel",
                    ));
                }
                _ if let Err(error) = resp.error_for_status() => {
                    txn.commit().await?;
                    return Err(MinilithEndpointError::internal_error(
                        "l1: transaction cancel failed!",
                        error,
                    ));
                }
                _ => {}
            },
            Err(error) => {
                txn.commit().await?;
                return Err(MinilithEndpointError::internal_error(
                    "failed to cancel transaction'",
                    error,
                ));
            }
        }

        sqlx::query!(
            "update ticket_reservations set transaction_id = null where user_id = $1",
            user.get_id()
        )
        .execute(&mut txn.executor())
        .await?;
    }

    // ========
    // remove old addons
    // ========
    sqlx::query!(
        "delete from ticket_reservation_addons where ticket_id = $1",
        reservation.id
    )
    .execute(&mut txn.executor())
    .await?;

    // we can't insert `unnest($1::integer[][])` for selected_options because postgres is weird
    // and represents 2D-arrays as a 1D array it'd get ugly
    for addon in &body.addons {
        sqlx::query!(
            "insert into ticket_reservation_addons
                (addon_id, ticket_id, selected_options, selected_text)
                values ($1, $2, $3, $4)",
            addon.id,
            reservation.id,
            addon.selected_options.as_deref().unwrap_or(&[]),
            addon.selected_text.as_deref().unwrap_or(""),
        )
        .execute(&mut txn.executor())
        .await?;
    }

    // ========
    // prepare Ware:s for transaction API
    // ========
    let lang = sqlx::query_scalar!("select language from users where id = $1", user.get_id())
        .fetch_one(&mut txn.executor())
        .await?;
    let lang = ctx
        .decrypt_string(lang)
        .wrap_err_encryption("failed to decrypt user language")?;

    let ticket_kind_name = reservation
        .ticket_kind_name
        .resolve_intl(&lang, "<ticket kind>");
    let activity_title = reservation.activity_title.resolve_intl(&lang, "<activity>");

    // we don't need to include ticket_kind because the ticket_addon_id is also a UUID so it
    // will never be duplicate!
    let available_addons = sqlx::query!(
        "select id, name as \"name!: DIS\", idx
            from ticket_addons
            where id = any($1)",
        &body.addons.iter().map(|addon| addon.id).collect::<Vec<_>>()
    )
    .fetch_all(&mut txn.executor())
    .await?;

    let mut transaction_wares = vec![transactions::Ware {
        name: format!("{activity_title} - {ticket_kind_name}"),
        amount: reservation.price.0,
        tax: 1.0,
        currency: transactions::Currency::Sek,
    }];
    let get_addon_idx = |id: Uuid| {
        available_addons
            .iter()
            .find(|addon| addon.id == id)
            .map_or(0, |addon| addon.idx)
    };
    // these got shuffled by `validate_addons`.
    body.addons
        .sort_unstable_by_key(|addon| get_addon_idx(addon.id));
    for addon in &body.addons {
        let info = available_addons
            .iter()
            .find(|available| available.id == addon.id)
            .wrap_err_internal(
                "we previously guaranteed (I though) that all options \
                    were in the DB and loaded. They were not.",
            )?;
        let addon_name = info.name.resolve_intl(&lang, "<addon>");
        // closure move bullshit, apparently we can't just move some variables...
        let lang = lang.clone();
        let options = chosen_options
            .iter()
            .filter(|opt| opt.ticket_addon_id == addon.id)
            .map(move |opt| {
                let option_name = opt.name.resolve_intl(&lang, "<option>");
                transactions::Ware {
                    name: format!("    {addon_name} - {option_name}"),
                    amount: opt.price,
                    tax: 1.0,
                    currency: transactions::Currency::Sek,
                }
            });
        transaction_wares.extend(options);
    }

    // ========
    // Check provider
    // ========
    let total_amount = transaction_wares
        .iter()
        .fold(0, |acc, ware| acc + ware.amount);
    if body.provider == PurchaseProvider::Free && total_amount != 0 {
        return Err(MinilithEndpointError::bad_frontend_code(
            "cannot pay for non-free ticket with free provider",
            "",
        ));
    }
    let provider = if total_amount == 0 {
        PurchaseProvider::Free
    } else {
        body.provider
    };

    // ========
    // Set UUID
    // ========
    sqlx::query!(
        "update ticket_reservations set transaction_id = $1
            where id = $2",
        transaction_id,
        reservation.id
    )
    .execute(&mut txn.executor())
    .await?;

    txn.commit().await?;

    // ========
    // Send transaction API request
    // ========
    let timeout = reservation
        .timeout
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .wrap_err_internal("failed to format value which we got & serialized before")?;
    let payment_req = transactions::CreatePaymentRequest {
        id: transaction_id,
        customer_id: Some(user.get_id().to_owned()),
        timeout,
        wares: transaction_wares,
        stripe_success_url: body.stripe_success_url,
    };
    let url = match provider {
        PurchaseProvider::Free => "/v0/free",
        PurchaseProvider::Swish => "/v0/swish",
        PurchaseProvider::Stripe => "/v0/stripe",
    };
    let resp = match ctx.transactions_post(url).json(&payment_req).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return Err(MinilithEndpointError::internal_error(
                "failed to buy ticket due to connection issues",
                err,
            ));
        }
    };
    if !resp.status().is_success() {
        return Err(MinilithEndpointError::internal_error(
            "failed to start transaction due to us being bad",
            (resp.status(), resp.text().await),
        ));
    }
    // ========
    // Handle transaction API response
    // ========
    let response = match provider {
        PurchaseProvider::Free => BuyTicketResponse {
            payment_request_token: None,
            stripe_url: None,
        },
        // these have to be separate match arms because the response type is different
        PurchaseProvider::Swish => {
            let body = resp
                .json::<transactions::CreatePaymentResponseSwish>()
                .await
                .wrap_err_internal("failed to start transaction due to us being bad in parsing")?;
            BuyTicketResponse {
                payment_request_token: Some(body.payment_request_token),
                stripe_url: None,
            }
        }
        PurchaseProvider::Stripe => {
            let body = resp
                .json::<transactions::CreatePaymentResponseStripe>()
                .await
                .wrap_err_internal("failed to start transaction due to us being bad in parsing")?;
            BuyTicketResponse {
                payment_request_token: None,
                stripe_url: Some(body.redirect_url),
            }
        }
    };

    Ok(response)
}

pub(super) async fn callback(
    ctx: &ContextWrapper,
    events: fed_auth_verifier::callbacks::TransactionsCallbackDataV1,
) -> MinilithResult<()> {
    for data in &*events {
        match data.inner.state {
            TransactionState::Pending => {}
            TransactionState::Paid => {
                let mut txn = ctx.db.begin().await?;
                if pay_for_reservation(&mut txn, data.transaction_id)
                    .await?
                    .is_some()
                {
                    txn.commit().await?;
                } else {
                    txn.rollback().await?;
                }
            }
            TransactionState::Refunded => {
                let affected = sqlx::query!(
                    "update purchased_tickets set owner_id = 'refunded:'
                        where transaction_id = $1",
                    data.transaction_id
                )
                .execute(&ctx.db)
                .await?;
                if affected.rows_affected() != 1 {
                    alert(AlertLevel::L1, "1 row not affected when purchase refunded!");
                    error!(transaction_id=%data.transaction_id,
                        "1 row not affected when purchase refunded!"
                    );
                }
            }
            TransactionState::Cancelled => {
                let mut txn = ctx.db.begin().await?;
                // if a cancel operation is locking the row, we ignore that and cancel it either
                // way
                let Some(flow) =
                    wait_for_user_purchase_flow_on_transaction_id(&mut txn, data.transaction_id)
                        .await?
                else {
                    continue;
                };
                if *flow != PurchaseFlow::Reservation {
                    return Err(MinilithEndpointError::internal_error(
                        "cancelled transaction did not have a reservation purchase flow",
                        flow,
                    ));
                }
                let Some(row) = sqlx::query!(
                    "update ticket_reservations
                        set transaction_id = null
                        where transaction_id = $1
                        returning
                            id,
                            user_id,
                            ticket_kind_id,
                            timeout < now() as \"has_timed_out!\"",
                    data.transaction_id,
                )
                .fetch_optional(&mut txn.executor())
                .await?
                else {
                    // it's gone, yipee
                    continue;
                };
                if row.has_timed_out {
                    sqlx::query!("delete from ticket_reservations where id = $1", row.id)
                        .execute(&mut txn.executor())
                        .await?;
                    sqlx::query!(
                        r#"update ticket_kinds
                            set reserved_or_purchased_tickets =
                                reserved_or_purchased_tickets - 1
                            where id = $1"#,
                        row.ticket_kind_id,
                    )
                    .execute(&mut txn.executor())
                    .await?;
                    unlist_user_purchase_flow(&mut txn, &row.user_id).await?;
                }
                txn.commit().await?;
                if row.has_timed_out {
                    drop(
                        give_reservations_in_new_transaction(&ctx.db, row.ticket_kind_id, 1).await,
                    );
                }
            }
        }
    }
    Ok(())
}

async fn check_missing_paid_reservation(
    txn: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<()> {
    let exists_purchased_ticket = sqlx::query_scalar!(
        "select exists (
            select 1 from purchased_tickets where transaction_id = $1
        ) as \"exists!\"",
        transaction_id
    )
    .fetch_one(&mut txn.executor())
    .await?;
    if !exists_purchased_ticket {
        error!(%transaction_id, "tried to pay for an unaccounted-for ticket");
        alert(AlertLevel::L1, "tried to pay for an unknown ticket");
    }
    Ok(())
}

/// # Returns
///
/// Returns the id of the ticket, or `None` for an already processed callback.
#[allow(
    clippy::too_many_lines,
    reason = "keeps the flow lock, ticket creation, and reservation cleanup atomic and ordered"
)]
async fn pay_for_reservation(
    txn: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<Option<Uuid>> {
    let Some(flow) =
        invalidate_wait_for_user_purchase_flow_on_transaction_id(txn, transaction_id).await?
    else {
        // A concurrent callback may have completed while this one waited.
        check_missing_paid_reservation(txn, transaction_id).await?;
        return Ok(None);
    };
    if *flow != PurchaseFlow::Reservation {
        return Err(MinilithEndpointError::internal_error(
            "paid transaction did not have a reservation purchase flow",
            flow,
        ));
    }

    let row = sqlx::query!(
        "select reservation.id, user_id, ticket_kind_id
        from ticket_reservations reservation
        where transaction_id = $1
        for update of reservation",
        transaction_id
    )
    .fetch_optional(&mut txn.executor())
    .await?
    .ok_or_else(|| {
        MinilithEndpointError::internal_error(
            "reservation disappeared while its purchase flow was locked",
            transaction_id,
        )
    })?;
    sqlx::query!(
        r#"update ticket_kinds
        set has_been_purchased = true
        where id = $1"#,
        row.ticket_kind_id,
    )
    .execute(&mut txn.executor())
    .await?;

    let purchased_id = sqlx::query_scalar!(
        "insert into purchased_tickets
        (id, purchaser_id, owner_id, ticket_kind_id, transaction_id)
        values ($1, $2, $2, $3, $4)
        returning id",
        row.id,
        row.user_id,
        row.ticket_kind_id,
        transaction_id
    )
    .fetch_one(&mut txn.executor())
    .await?;
    if purchased_id != row.id {
        return Err(MinilithEndpointError::internal_error(
            "purchased ticket id differed from its reservation id",
            purchased_id,
        ));
    }
    // move addons:
    sqlx::query!(
        "insert into purchased_ticket_addons
        (addon_id, ticket_id, selected_options, selected_text)
        select addon_id, ticket_id, selected_options, selected_text
        from ticket_reservation_addons
            where ticket_id = $1",
        row.id,
    )
    .execute(&mut txn.executor())
    .await?;
    sqlx::query!(
        "delete from ticket_reservation_addons
            where ticket_id = $1",
        row.id,
    )
    .execute(&mut txn.executor())
    .await?;
    // end move addons

    let affected = sqlx::query!(
        "delete from ticket_reservations where transaction_id = $1",
        transaction_id
    )
    .execute(&mut txn.executor())
    .await?;
    if affected.rows_affected() != 1 {
        error!(%transaction_id,
            "1 row not affected when purchase complete!"
        );
        alert(AlertLevel::L1, "1 row not affected when purchase complete!");
        return Ok(None);
    }
    unlist_user_purchase_flow(txn, &row.user_id).await?;
    Ok(Some(row.id))
}

struct ReturnedAddonOption {
    ticket_addon_id: Uuid,
    name: DIS,
    price: i64,
}

/// Ensure that the addons aren't duplicated and that they belong to the
/// specified `ticket_kind`.
#[allow(
    clippy::too_many_lines,
    reason = "it's quite linear and does a single function"
)]
async fn validate_addons(
    txn: &mut Transaction<'_>,
    addons: &mut [BoughtAddon],
    ticket_kind: Uuid,
) -> MinilithResult<Vec<ReturnedAddonOption>> {
    addons.sort_unstable_by_key(|addon| addon.id);
    addons
        .iter()
        .zip(addons.iter().skip(1))
        .try_for_each(|(one_of_them, the_one_after)| {
            if one_of_them.id == the_one_after.id {
                Err(MinilithEndpointError::bad_frontend_code(
                    format!("addon {} is duplicated", one_of_them.id),
                    "",
                ))
            } else {
                Ok(())
            }
        })?;

    let addon_ids = addons.iter().map(|addon| addon.id).collect::<Vec<_>>();
    let addon_data = sqlx::query!(
        "select has_text_field, required, multiple_alternatives,
        name as \"name!: DIS\"
        from unnest($1::uuid[]) as t(id) 
        inner join ticket_addons on ticket_addons.id = t.id
            and ticket_kind_id = $2
        order by t.id",
        &addon_ids,
        ticket_kind
    )
    .fetch_all(&mut txn.executor())
    .await?;
    if addon_data.len() != addons.len() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "not all addons exist!",
            "",
        ));
    }

    // pairwise we need to verify these
    // they have the same order
    let selected_options_ids = addons
        .iter()
        .flat_map(|addon| addon.selected_options.iter().flatten().map(|_| addon.id))
        .collect::<Vec<_>>();
    let selected_options_idxes = addons
        .iter()
        .flat_map(|addon| addon.selected_options.iter().flatten())
        .copied()
        .collect::<Vec<_>>();

    let valid_indices = sqlx::query_as!(
        ReturnedAddonOption,
        "with input as (
            select ticket_addon_id, idx from
            unnest($1::uuid[], $2::integer[]) as t(ticket_addon_id, idx)
        )
        select opts.ticket_addon_id, name as \"name!: DIS\", price as \"price!: i64\"
        from input
        inner join ticket_addon_options opts
            on (opts.ticket_addon_id = input.ticket_addon_id and opts.idx = input.idx)",
        &selected_options_ids,
        &selected_options_idxes
    )
    .fetch_all(&mut txn.executor())
    .await?;

    if selected_options_ids.len() != valid_indices.len() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "selected_options contains some indices which were not valid",
            "",
        ));
    }

    for (addon, row) in addons.iter_mut().zip(addon_data.iter()) {
        if !row.has_text_field && addon.selected_text.is_some() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "addon has text even though this is not allowed",
                "",
            ));
        }
        let n_options = usize::from(
            row.has_text_field
                && addon
                    .selected_text
                    .as_ref()
                    .is_some_and(|text| !text.trim().is_empty()),
        ) + addon.selected_options.as_ref().map_or(0, Vec::len);

        if row.required && n_options == 0 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "required addon missing option",
                "",
            ));
        }
        if !row.multiple_alternatives && n_options > 1 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "too many selected options! Only 1 is permitted",
                "",
            ));
        }

        if let Some(options) = &mut addon.selected_options {
            let before_len = options.len();
            options.sort_unstable();
            options.dedup();
            if options.len() != before_len {
                return Err(MinilithEndpointError::bad_frontend_code(
                    format!(
                        "duplicate option for addon {}",
                        row.name.resolve_intl("en", "")
                    ),
                    "",
                ));
            }
        }
    }

    Ok(valid_indices)
}
