use bin_common::Transaction;
use fed_auth_verifier::{User, callbacks::TransactionState};
use minilith_errors::{
    AlertLevel, MinilithErrorOptionExt as _, MinilithErrorResultExt as _, alert,
};
use sqlx::postgres::types::PgMoney;
use sqlx::types::time::OffsetDateTime;
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
    if body.provider == PurchaseProvider::Sumup && body.redirect_url.is_none() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "redirect_url has to be non-null when provider is sumup!",
            "",
        ));
    }

    // ========
    // Get UUID (first, so we don't hog a DB connection)
    // ========
    let transaction_id = Uuid::new_v4();

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
        "select product_reservations.id, product_reservations.product_id, timeout, transaction_id,
                kind.name as \"ticket_kind_name!: DIS\",
                activities.title as \"activity_title?: DIS\",
                kind.sales_price as \"price!\", activities.id as \"activity_id?\"
            from product_reservations
            inner join products kind on (kind.id = product_reservations.product_id)
            -- A store product belongs to no activity; one that does is sold only until the
            -- activity has ended.
            left join product_activity_assignments assignment on assignment.product_id = kind.id
            left join activities on (activities.id = assignment.activity_id)
            where user_id = $1
            and (assignment.activity_id is null or activities.time_end > now())",
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
            "update product_reservations set transaction_id = null where user_id = $1",
            user.get_id()
        )
        .execute(&mut txn.executor())
        .await?;
    }

    // ========
    // remove old addons
    // ========
    sqlx::query!(
        "delete from product_reservation_addons where reservation_id = $1",
        reservation.id
    )
    .execute(&mut txn.executor())
    .await?;

    // we can't insert `unnest($1::integer[][])` for selected_options because postgres is weird
    // and represents 2D-arrays as a 1D array it'd get ugly
    for addon in &body.addons {
        sqlx::query!(
            "insert into product_reservation_addons
                (addon_id, reservation_id, selected_options, selected_options_prices, selected_text)
                values ($1, $2, $3,
                    (select coalesce(array_agg(coalesce(opt.price, 0::money) order by chosen.ord), '{}'::money[])
                        from unnest($3::integer[]) with ordinality as chosen(idx, ord)
                        left join product_addon_options opt
                            on opt.product_addon_id = $1 and opt.idx = chosen.idx),
                    $4)",
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
    let activity_title = reservation
        .activity_title
        .as_ref()
        .map(|title| title.resolve_intl(&lang, "<activity>"));

    // we don't need to include ticket_kind because the ticket_addon_id is also a UUID so it
    // will never be duplicate!
    let available_addons = sqlx::query!(
        "select id, name as \"name!: DIS\", idx
            from product_addons
            where id = any($1)",
        &body.addons.iter().map(|addon| addon.id).collect::<Vec<_>>()
    )
    .fetch_all(&mut txn.executor())
    .await?;

    let mut transaction_wares = vec![transactions::Ware {
        // A store product's line is just its name.
        name: match &activity_title {
            Some(activity_title) => format!("{activity_title} - {ticket_kind_name}"),
            None => ticket_kind_name.to_owned(),
        },
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
            .filter(|opt| opt.product_addon_id == addon.id)
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
        "update product_reservations set transaction_id = $1
            where id = $2",
        transaction_id,
        reservation.id
    )
    .execute(&mut txn.executor())
    .await?;

    txn.commit().await?;

    let timeout = reservation
        .timeout
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .wrap_err_internal("failed to format value which we got & serialized before")?;
    let payment_req = transactions::CreatePaymentRequest {
        id: transaction_id,
        customer_id: Some(user.get_id().to_owned()),
        timeout,
        wares: transaction_wares,
        redirect_url: body.redirect_url,
    };
    send_payment(ctx, provider, payment_req).await
}

/// Sends a prepared payment to the transactions API and normalizes its response.
async fn send_payment(
    ctx: &ContextWrapper,
    provider: PurchaseProvider,
    payment_req: transactions::CreatePaymentRequest,
) -> MinilithResult<BuyTicketResponse> {
    let url = match provider {
        PurchaseProvider::Free => "/v0/free",
        PurchaseProvider::Swish => "/v0/swish",
        PurchaseProvider::Sumup => "/v0/sumup",
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
    match provider {
        PurchaseProvider::Free => Ok(BuyTicketResponse {
            payment_request_token: None,
            redirect_url: None,
        }),
        // these have to be separate match arms because the response type is different
        PurchaseProvider::Swish => {
            let body = resp
                .json::<transactions::CreatePaymentResponseSwish>()
                .await
                .wrap_err_internal("failed to start transaction due to us being bad in parsing")?;
            Ok(BuyTicketResponse {
                payment_request_token: Some(body.payment_request_token),
                redirect_url: None,
            })
        }
        PurchaseProvider::Sumup => {
            let body = resp
                .json::<transactions::CreatePaymentResponseSumup>()
                .await
                .wrap_err_internal("failed to start transaction due to us being bad in parsing")?;
            Ok(BuyTicketResponse {
                payment_request_token: None,
                redirect_url: body.redirect_url,
            })
        }
    }
}

/// Asks the transactions service what its fee would be for this provider, amount & kind.
pub(crate) async fn fee_quote(
    ctx: &ContextWrapper,
    provider: PurchaseProvider,
    amount: i64,
    kind: transactions::FeeKind,
) -> MinilithResult<i64> {
    let provider = match provider {
        PurchaseProvider::Swish => transactions::FeeProvider::Swish,
        PurchaseProvider::Sumup => transactions::FeeProvider::Sumup,
        PurchaseProvider::Free => return Ok(0),
    };
    let resp = ctx
        .transactions_post("/v0/fee-quote")
        .json(&transactions::FeeQuoteRequest {
            provider,
            amount,
            physical: false,
            kind,
        })
        .send()
        .await
        .wrap_err_internal("fee quote transport failed")?;
    if !resp.status().is_success() {
        return Err(MinilithEndpointError::internal_error(
            "fee quote failed",
            (resp.status(), resp.text().await),
        ));
    }
    Ok(resp
        .json::<transactions::FeeQuoteResponse>()
        .await
        .wrap_err_internal("fee quote parse failed")?
        .fee)
}

/// Refunds a purchase in full through the payment provider and invalidates it.
///
/// Ownership is left unchanged; `refunded_id` marks the purchase as refunded, which use,
/// transfer and resale reject. The caller must have established product adminship.
#[allow(
    clippy::too_many_lines,
    reason = "keeps the refund claim and provider sequence visible in one place"
)]
pub(crate) async fn refund(ctx: &ContextWrapper, purchase_id: Uuid) -> MinilithResult<()> {
    let purchase = sqlx::query!(
        "select owner_id, refunded_id, resale_transaction_id,
            for_resale as \"for_resale?: PgMoney\"
        from purchases where id = $1",
        purchase_id
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;
    if purchase.resale_transaction_id.is_some() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "a resale payment is in progress for this purchase",
            "",
        ));
    }
    let transaction_id = sqlx::query_scalar!(
        "select transaction_id from purchase_transactions
        where purchase_id = $1 and buyer_id = $2 and transaction_id is not null
        order by completed_at desc limit 1",
        purchase_id,
        &purchase.owner_id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .flatten()
    .ok_or_else(|| {
        MinilithEndpointError::bad_frontend_code(
            "the purchase has no paid transaction to refund",
            "",
        )
    })?;
    // `refunded_id` is only set by a refund of the current owner's payment, so any value means
    // this purchase is already refunded.
    if purchase.refunded_id.is_some() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "this purchase has already been refunded",
            "",
        ));
    }
    let info = ctx
        .transactions_post("/v0/info")
        .json(&transactions::InfoRequest {
            transaction_ids: vec![transaction_id],
        })
        .send()
        .await
        .wrap_err_internal("failed to read the purchase transaction")?
        .error_for_status()
        .wrap_err_internal("failed to read the purchase transaction")?
        .json::<Vec<transactions::SingleInfoResponse>>()
        .await
        .wrap_err_internal("failed to parse the purchase transaction")?
        .into_iter()
        .next()
        .ok_or_else(|| {
            MinilithEndpointError::internal_error("missing purchase transaction info", "")
        })?;
    if !matches!(
        info.state,
        TransactionState::Paid | TransactionState::Refunded
    ) {
        return Err(MinilithEndpointError::bad_frontend_code(
            "only a paid purchase can be refunded",
            info.state,
        ));
    }

    // Claim the refund before calling the provider so a concurrent refund cannot also pass.
    let claimed = sqlx::query!(
        "update purchases
        set refunded_id = $2, for_resale = null
        where id = $1 and owner_id = $3
        and refunded_id is null
        and resale_transaction_id is null",
        purchase_id,
        transaction_id,
        &purchase.owner_id,
    )
    .execute(&ctx.db)
    .await?;
    if claimed.rows_affected() != 1 {
        return Err(MinilithEndpointError::bad_frontend_code(
            "the purchase changed while it was being refunded",
            "",
        ));
    }
    // A free purchase has no money to return; invalidating it above is the whole refund.
    if matches!(info.state, TransactionState::Refunded)
        || matches!(info.provider, Some(transactions::Provider::Free))
    {
        return Ok(());
    }

    let response = match ctx
        .transactions_post(format!("/v0/{transaction_id}/refund"))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            // Keep the claim: the refund might still have reached the provider.
            alert(AlertLevel::L1, "purchase refund transport failed");
            error!(?error, "purchase refund transport failed");
            return Err(MinilithEndpointError::internal_error(
                "l1: purchase refund transport failed",
                error,
            ));
        }
    };
    if !response.status().is_success() {
        // A rejection means no refund happened, so release the claim and restore the resale state.
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        sqlx::query!(
            "update purchases
            set refunded_id = $2, for_resale = $4
            where id = $1 and refunded_id = $3",
            purchase_id,
            purchase.refunded_id,
            transaction_id,
            purchase.for_resale,
        )
        .execute(&ctx.db)
        .await?;
        return Err(MinilithEndpointError::bad_frontend_code(
            "the payment refund was rejected",
            (status, body),
        ));
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps the resale validation and payment sequence visible in one place"
)]
pub(super) async fn begin_resale(
    ctx: &ContextWrapper,
    user: User,
    purchase_id: Uuid,
    provider: PurchaseProvider,
    redirect_url: Option<String>,
) -> MinilithResult<BuyTicketResponse> {
    let purchase = sqlx::query!(
        r#"select purchase.product_id, purchase.owner_id,
            purchase.for_resale as "for_resale?: PgMoney",
            product.name as "product_name!: DIS",
            (now() between product.resale_start and product.resale_stop)
                as "within_resale_window!",
            purchase.resale_transaction_id,
            (purchase.refunded_id is not null) as "refunded!"
        from purchases purchase
        inner join products product on product.id = purchase.product_id
        where purchase.id = $1"#,
        purchase_id
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;
    let Some(price) = purchase.for_resale else {
        return Err(MinilithEndpointError::bad_frontend_code(
            "this purchase is not for sale",
            "",
        ));
    };
    if purchase.refunded {
        return Err(MinilithEndpointError::bad_frontend_code(
            "this purchase has been refunded",
            "",
        ));
    }
    if purchase.owner_id == user.get_id() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "you already own this purchase",
            "",
        ));
    }
    if !purchase.within_resale_window {
        return Err(MinilithEndpointError::bad_frontend_code(
            "resale is closed for this product",
            "",
        ));
    }
    if purchase.resale_transaction_id.is_some() {
        return Err(MinilithEndpointError::bad_frontend_code(
            "someone else is already buying this",
            "",
        ));
    }
    super::access::ensure_user_may_purchase_ticket(&ctx.db, user.get_id(), purchase.product_id)
        .await?;

    // The transaction the seller originally paid with is the one that gets refunded.
    let seller_transaction_id = sqlx::query_scalar!(
        "select transaction_id from purchase_transactions
        where purchase_id = $1
        and buyer_id = $2
        and transaction_id is not null
        order by completed_at desc
        limit 1",
        purchase_id,
        &purchase.owner_id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .flatten()
    .ok_or_else(|| {
        MinilithEndpointError::bad_frontend_code(
            "l1: the seller has no paid transaction to refund",
            "",
        )
    })?;

    // A free resale moves the purchase without money changing hands, so no fees are charged.
    // Only the costs this resale creates are charged to the buyer: the refund of the seller's
    // transaction and the new payment. The seller's original payment fee was already kept on the
    // original sale (`complete_resale` refunds the resale price, not their whole payment), so
    // charging it again would collect it once per resale.
    let (refund_fee, new_fee) = if price.0 == 0 {
        (0, 0)
    } else {
        let info = ctx
            .transactions_post("/v0/info")
            .json(&transactions::InfoRequest {
                transaction_ids: vec![seller_transaction_id],
            })
            .send()
            .await
            .wrap_err_internal("failed to read the seller's transaction")?
            .error_for_status()
            .wrap_err_internal("failed to read the seller's transaction")?
            .json::<Vec<transactions::SingleInfoResponse>>()
            .await
            .wrap_err_internal("failed to parse the seller's transaction")?;
        // Refunding costs us a fee on the seller's provider (flat on `Swish`, free on `SumUp`).
        let refund_fee = match info.first().and_then(|info| info.provider) {
            Some(transactions::Provider::Swish) => {
                fee_quote(
                    ctx,
                    PurchaseProvider::Swish,
                    price.0,
                    transactions::FeeKind::Refund,
                )
                .await?
            }
            Some(transactions::Provider::Sumup) => {
                fee_quote(
                    ctx,
                    PurchaseProvider::Sumup,
                    price.0,
                    transactions::FeeKind::Refund,
                )
                .await?
            }
            Some(transactions::Provider::Free | transactions::Provider::Stripe) | None => 0,
        };
        let new_fee = fee_quote(ctx, provider, price.0, transactions::FeeKind::Payment).await?;
        (refund_fee, new_fee)
    };
    let total_fees = refund_fee + new_fee;
    let total = price.0 + total_fees;
    let provider = if total == 0 {
        PurchaseProvider::Free
    } else {
        provider
    };
    if provider == PurchaseProvider::Free && total != 0 {
        return Err(MinilithEndpointError::bad_frontend_code(
            "cannot pay for a non-free resale with the free provider",
            "",
        ));
    }

    let lang = sqlx::query_scalar!("select language from users where id = $1", user.get_id())
        .fetch_one(&ctx.db)
        .await?;
    let lang = ctx
        .decrypt_string(lang)
        .wrap_err_encryption("failed to decrypt user language")?;
    let product_name = purchase.product_name.resolve_intl(&lang, "<product>");

    let mut wares = vec![transactions::Ware {
        name: format!("{product_name} (resale)"),
        amount: price.0,
        tax: 1.0,
        currency: transactions::Currency::Sek,
    }];
    if total_fees > 0 {
        wares.push(transactions::Ware {
            name: "Resale transaction fee".to_owned(),
            amount: total_fees,
            tax: 1.0,
            currency: transactions::Currency::Sek,
        });
    }

    let transaction_id = Uuid::new_v4();
    let reserved = sqlx::query!(
        "update purchases
        set resale_transaction_id = $2, resale_buyer_id = $3
        where id = $1
        and owner_id = $4
        and for_resale is not null
        and resale_transaction_id is null",
        purchase_id,
        transaction_id,
        user.get_id(),
        &purchase.owner_id,
    )
    .execute(&ctx.db)
    .await?;
    if reserved.rows_affected() != 1 {
        return Err(MinilithEndpointError::bad_frontend_code(
            "someone else is already buying this",
            "",
        ));
    }
    let timeout = (OffsetDateTime::now_utc() + time::Duration::minutes(20))
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .wrap_err_internal("failed to format the resale timeout")?;
    let result = send_payment(
        ctx,
        provider,
        transactions::CreatePaymentRequest {
            id: transaction_id,
            customer_id: Some(user.get_id().to_owned()),
            timeout,
            wares,
            redirect_url,
        },
    )
    .await;
    if result.is_err() {
        sqlx::query!(
            "update purchases
            set resale_transaction_id = null, resale_buyer_id = null
            where resale_transaction_id = $1",
            transaction_id
        )
        .execute(&ctx.db)
        .await?;
    }
    result
}

struct CompletedResale {
    seller_transaction_id: Uuid,
    refund_amount: i64,
}

/// Completes a paid resale by moving ownership and appending a purchase transaction.
/// Add-ons are deliberately left untouched.
#[allow(
    clippy::too_many_lines,
    reason = "keeps the resale completion atomic and auditable"
)]
async fn complete_resale(
    txn: &mut Transaction<'_>,
    transaction_id: Uuid,
) -> MinilithResult<Option<CompletedResale>> {
    let Some(pending) = sqlx::query!(
        "select id, owner_id, resale_buyer_id,
            resale_buyer_id as \"buyer_id!\",
            owner_id as \"seller_id!\",
            for_resale as \"price!: PgMoney\",
            paid_price as \"paid_price!: PgMoney\"
        from purchases
        where resale_transaction_id = $1
        for update",
        transaction_id
    )
    .fetch_optional(&mut txn.executor())
    .await?
    else {
        return Ok(None);
    };
    let seller_transaction_id = sqlx::query_scalar!(
        "select transaction_id from purchase_transactions
        where purchase_id = $1
        and buyer_id = $2
        and transaction_id is not null
        order by completed_at desc
        limit 1",
        pending.id,
        &pending.seller_id,
    )
    .fetch_optional(&mut txn.executor())
    .await?
    .flatten()
    .wrap_err_internal("the seller's transaction disappeared while completing a resale")?;
    let affected = sqlx::query!(
        "update purchases
        set owner_id = $2, for_resale = null, purchased_at = now(),
            paid_price = $4, resale_transaction_id = null, resale_buyer_id = null
        where id = $1 and for_resale is not null and owner_id = $3",
        pending.id,
        &pending.buyer_id,
        &pending.seller_id,
        pending.price,
    )
    .execute(&mut txn.executor())
    .await?;
    if affected.rows_affected() != 1 {
        return Err(MinilithEndpointError::internal_error(
            "the resale was no longer available when its payment completed",
            transaction_id,
        ));
    }
    sqlx::query!(
        "insert into purchase_transactions
        (purchase_id, seller_id, buyer_id, completed_at, transaction_id)
        values ($1, $2, $3, now(), $4)",
        pending.id,
        &pending.seller_id,
        &pending.buyer_id,
        transaction_id,
    )
    .execute(&mut txn.executor())
    .await?;
    Ok(Some(CompletedResale {
        seller_transaction_id,
        // A listing from before `paid_price` existed (or a chain that resold above cost) could
        // exceed what the seller actually paid; never ask the provider for more than was captured.
        refund_amount: pending.price.0.min(pending.paid_price.0),
    }))
}

#[allow(
    clippy::too_many_lines,
    reason = "one match over every transaction state"
)]
pub(crate) async fn callback(
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
                } else if let Some(resale) = complete_resale(&mut txn, data.transaction_id).await? {
                    txn.commit().await?;
                    // A free resale refunds nothing; `/v0/{id}/refund/0` would be rejected.
                    if resale.refund_amount == 0 {
                        continue;
                    }
                    let url = format!(
                        "/v0/{}/refund/{}",
                        resale.seller_transaction_id, resale.refund_amount
                    );
                    if let Err(error) = ctx.transactions_post(url).send().await {
                        alert(AlertLevel::L1, "resale refund transport failed");
                        error!(?error, "resale refund transport failed");
                    }
                } else {
                    txn.rollback().await?;
                }
            }
            TransactionState::Refunded => {
                // Only the current owner's latest transaction is the purchase itself. A resale
                // refunds the previous owner's older transaction and must not invalidate the
                // purchase (which is why `refunded_id` is only written for admin refunds).
                let affected = sqlx::query!(
                    "update purchases
                        set refunded_id = $1
                        where id in (
                            select purchase_id from purchase_transactions
                            where transaction_id = $1
                        )
                        and $1 = (select transaction_id from purchase_transactions latest
                            where latest.purchase_id = purchases.id
                            and latest.buyer_id = purchases.owner_id
                            and latest.transaction_id is not null
                            order by latest.completed_at desc limit 1)",
                    data.transaction_id,
                )
                .execute(&ctx.db)
                .await?;
                if affected.rows_affected() != 1 {
                    // A known older transaction is the seller's payment being refunded as part of
                    // a resale; that is not a refund of the purchase.
                    let known = sqlx::query_scalar!(
                        "select exists (select 1 from purchase_transactions
                            where transaction_id = $1) as \"exists!\"",
                        data.transaction_id,
                    )
                    .fetch_one(&ctx.db)
                    .await?;
                    if !known {
                        alert(AlertLevel::L1, "1 row not affected when purchase refunded!");
                        error!(transaction_id=%data.transaction_id,
                            "1 row not affected when purchase refunded!"
                        );
                    }
                }
            }
            TransactionState::Cancelled => {
                let mut txn = ctx.db.begin().await?;
                let cleared = sqlx::query!(
                    "update purchases
                    set resale_transaction_id = null, resale_buyer_id = null
                    where resale_transaction_id = $1",
                    data.transaction_id
                )
                .execute(&mut txn.executor())
                .await?;
                if cleared.rows_affected() == 1 {
                    txn.commit().await?;
                    continue;
                }
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
                    "update product_reservations
                        set transaction_id = null
                        where transaction_id = $1
                        returning
                            id,
                            user_id,
                            product_id,
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
                    sqlx::query!("delete from product_reservations where id = $1", row.id)
                        .execute(&mut txn.executor())
                        .await?;
                    sqlx::query!(
                        r#"update products
                            set sales_stock = sales_stock + 1
                            where id = $1 and sales_stock is not null"#,
                        row.product_id,
                    )
                    .execute(&mut txn.executor())
                    .await?;
                    unlist_user_purchase_flow(&mut txn, &row.user_id).await?;
                }
                txn.commit().await?;
                if row.has_timed_out {
                    drop(give_reservations_in_new_transaction(ctx, row.product_id, 1).await);
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
            select 1 from purchase_transactions where transaction_id = $1
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
        "select reservation.id, user_id, product_id, kind.sales_price as \"price!\",
            (select assignment.activity_id from product_activity_assignments assignment
                where assignment.product_id = kind.id
                order by assignment.activity_id limit 1) as \"activity_id?\"
        from product_reservations reservation
        inner join products kind on kind.id = reservation.product_id
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
        r#"update products
        set been_purchased = true
        where id = $1"#,
        row.product_id,
    )
    .execute(&mut txn.executor())
    .await?;

    let purchased_id = sqlx::query_scalar!(
        "insert into purchases
        (id, purchaser_id, owner_id, product_id, product_price, paid_price, activity_id,
            purchased_at)
        values ($1, $2, $2, $3, $4,
            $4 + coalesce((
                select sum(option_price)
                from product_reservation_addons addon
                cross join lateral unnest(addon.selected_options_prices) option_price
                where addon.reservation_id = $1
            ), '0.00'::money),
            $5, now())
        returning id",
        row.id,
        row.user_id,
        row.product_id,
        row.price,
        row.activity_id
    )
    .fetch_one(&mut txn.executor())
    .await?;
    sqlx::query!(
        "insert into purchase_transactions
        (purchase_id, seller_id, buyer_id, completed_at, transaction_id)
        values ($1, null, $2, now(), $3)",
        row.id,
        row.user_id,
        transaction_id
    )
    .execute(&mut txn.executor())
    .await?;
    if purchased_id != row.id {
        return Err(MinilithEndpointError::internal_error(
            "purchased ticket id differed from its reservation id",
            purchased_id,
        ));
    }
    // move addons:
    sqlx::query!(
        "insert into purchased_product_addons
        (addon_id, purchase_id, selected_options, selected_options_prices, selected_text)
        select addon_id, reservation_id, selected_options, selected_options_prices, selected_text
        from product_reservation_addons
            where reservation_id = $1",
        row.id,
    )
    .execute(&mut txn.executor())
    .await?;
    sqlx::query!(
        "delete from product_reservation_addons
            where reservation_id = $1",
        row.id,
    )
    .execute(&mut txn.executor())
    .await?;
    // end move addons

    let affected = sqlx::query!(
        "delete from product_reservations where transaction_id = $1",
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
    product_addon_id: Uuid,
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
        inner join product_addons on product_addons.id = t.id
        inner join product_addon_links links on links.addon_id = t.id
            and links.product_id = $2
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
            select product_addon_id, idx from
            unnest($1::uuid[], $2::integer[]) as t(product_addon_id, idx)
        )
        select opts.product_addon_id, name as \"name!: DIS\", price as \"price!: i64\"
        from input
        inner join product_addon_options opts
            on (opts.product_addon_id = input.product_addon_id and opts.idx = input.idx)",
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
