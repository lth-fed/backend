//! Purchases and sales reporting for admins: the `/purchases` view, receipts and receipt emails.
//!
//! Both the admin API and sales devices use the same [`load_purchases`] query and receipt
//! rendering; the two callers only differ in how they scope and authorize a purchase.

use std::collections::HashMap;
use std::ops::Deref;

use fed_auth_verifier::User;
use fed_auth_verifier::callbacks::TransactionState;
use minilith_errors::{
    MinilithEndpointError, MinilithErrorOptionExt as _, MinilithErrorResultExt as _,
};
use poem_openapi::param::{Path, Query};
use poem_openapi::payload::{Binary, Json, Response};
use poem_openapi::{Object, OpenApi};
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use crate::admin::AdminProductKind;
use crate::context::ContextWrapper;
use crate::group::admin::{check_activity_adminship, check_direct_adminship};
use crate::product::catalog::{owner_transaction, receipt_language, receipt_pdf};
use crate::{
    DbInternationalizedString as DIS, InternationalizedString, MinilithResult, transactions,
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

/// One purchase, as listed in the purchases view and on a sales device.
#[derive(Debug, Clone, Object)]
pub struct PurchaseRow {
    pub id: Uuid,
    pub product_id: Uuid,
    pub product_name: InternationalizedString,
    pub kind: AdminProductKind,
    /// The activity it was sold for, if any.
    pub activity_id: Option<Uuid>,
    pub purchased_at: OffsetDateTime,
    pub purchaser_id: String,
    pub purchaser_name: String,
    pub owner_id: String,
    pub owner_name: String,
    /// What the current owner paid (in ören).
    pub paid_price: i64,
    /// The current owner's resale listing price (in ören), if listed.
    pub for_resale: Option<i64>,
    /// Refunded purchases are not income and cannot be used.
    pub refunded: bool,
    /// How many times it has been used.
    pub uses: i64,
    pub max_uses: i32,
    /// Whether the current owner paid for it, i.e. whether a receipt exists.
    pub has_receipt: bool,
}

/// A product's sales over the selected interval.
#[derive(Debug, Clone, Object)]
pub struct SoldProduct {
    pub product_id: Uuid,
    pub name: InternationalizedString,
    pub kind: AdminProductKind,
    pub max_uses: i32,
    /// Purchases that were not refunded.
    pub sold: i64,
    pub refunded: i64,
    /// Sum of the purchase prices of the non-refunded purchases (in ören).
    pub revenue: i64,
    pub uses: i64,
    /// `max_uses` summed over the non-refunded purchases: the most it could have been used.
    pub uses_possible: i64,
}

/// What a caller may see: the products of a group, of an activity, or a device's products.
#[derive(Debug, Clone)]
pub struct PurchaseQuery {
    pub product_ids: Option<Vec<Uuid>>,
    pub group_id: Option<Uuid>,
    pub activity_id: Option<Uuid>,
    pub start: Option<OffsetDateTime>,
    pub end: Option<OffsetDateTime>,
}

/// Lists purchases matching `query`, newest first.
///
/// Callers must authorize the scope themselves; this only filters.
///
/// # Errors
///
/// `DB`.
pub async fn load_purchases(
    ctx: &ContextWrapper,
    query: PurchaseQuery,
) -> MinilithResult<Vec<PurchaseRow>> {
    let rows = sqlx::query!(
        r#"select
            purchases.id,
            purchases.product_id,
            products.name as "product_name!: DIS",
            products.kind::text as "kind!",
            products.max_uses,
            purchases.activity_id,
            purchases.purchased_at,
            purchases.purchaser_id,
            purchaser.name as "purchaser_name?",
            purchases.owner_id,
            owner.name as "owner_name?",
            purchases.paid_price as "paid_price!",
            purchases.for_resale as "for_resale?",
            (purchases.refunded_id is not null) as "refunded!",
            (select count(*) from purchase_uses uses
                where uses.purchase_id = purchases.id) as "uses!",
            exists (select 1 from purchase_transactions payment
                where payment.purchase_id = purchases.id
                and payment.buyer_id = purchases.owner_id
                and payment.transaction_id is not null) as "has_receipt!"
        from purchases
        inner join products on products.id = purchases.product_id
        left join users purchaser on purchaser.id = purchases.purchaser_id
        left join users owner on owner.id = purchases.owner_id
        where ($1::uuid[] is null or products.id = any($1))
        and ($2::uuid is null or products.group_id = $2)
        and ($3::uuid is null or purchases.activity_id = $3)
        and ($4::timestamptz is null or purchases.purchased_at >= $4)
        and ($5::timestamptz is null or purchases.purchased_at < $5)
        order by purchases.purchased_at desc, purchases.id"#,
        query.product_ids.as_deref(),
        query.group_id,
        query.activity_id,
        query.start,
        query.end,
    )
    .fetch_all(&ctx.db)
    .await?;
    let name = |name: Option<Vec<u8>>| {
        name.and_then(|name| ctx.decrypt_string(name))
            .unwrap_or_default()
    };
    let mut purchases = Vec::with_capacity(rows.len());
    for row in rows {
        purchases.push(PurchaseRow {
            id: row.id,
            product_id: row.product_id,
            product_name: row.product_name.0,
            kind: AdminProductKind::from_db(&row.kind)?,
            activity_id: row.activity_id,
            purchased_at: row.purchased_at,
            purchaser_id: row.purchaser_id,
            purchaser_name: name(row.purchaser_name),
            owner_id: row.owner_id,
            owner_name: name(row.owner_name),
            paid_price: row.paid_price.0,
            for_resale: row.for_resale.map(|price| price.0),
            refunded: row.refunded,
            uses: row.uses,
            max_uses: row.max_uses,
            has_receipt: row.has_receipt,
        });
    }
    Ok(purchases)
}

/// The fields the receipt endpoints need about a purchase.
pub(crate) struct PurchaseReceiptTarget {
    pub product_id: Uuid,
    group_id: Uuid,
    activity_id: Option<Uuid>,
    product_name: InternationalizedString,
    owner_name: String,
    owner_language: String,
}

impl PurchaseReceiptTarget {
    /// Renders the receipt of the purchase's current owner.
    pub(crate) async fn receipt(
        &self,
        ctx: &ContextWrapper,
        purchase_id: Uuid,
    ) -> MinilithResult<Vec<u8>> {
        let Some(transaction_id) = owner_transaction(ctx, purchase_id).await? else {
            return Err(MinilithEndpointError::bad_frontend_code(
                "the purchase was transferred and has no receipt",
                "",
            ));
        };
        receipt_pdf(
            ctx,
            transaction_id,
            receipt_language(&self.owner_language),
            &self.owner_name,
        )
        .await
    }

    /// Emails the receipt of the purchase's current owner to `email`.
    pub(crate) async fn email_receipt(
        &self,
        ctx: &ContextWrapper,
        purchase_id: Uuid,
        email: &str,
    ) -> MinilithResult<()> {
        if !email.contains('@') {
            return Err(MinilithEndpointError::bad_user_input(
                "email",
                "invalid email address",
                "",
                "email",
            ));
        }
        let Some(email_client) = ctx.email_client() else {
            return Err(MinilithEndpointError::bad_frontend_code(
                "sending email is not configured",
                "",
            ));
        };
        let pdf = self.receipt(ctx, purchase_id).await?;
        let swedish = matches!(
            receipt_language(&self.owner_language),
            transactions::Language::Swedish
        );
        let product_name = self
            .product_name
            .resolve_intl(if swedish { "sv" } else { "en" }, "");
        let (subject, body) = if swedish {
            (
                format!("Kvitto – {product_name}"),
                format!(
                    "<p>Här kommer kvittot för <strong>{product_name}</strong> i den bifogade \
                    PDF-filen.</p>"
                ),
            )
        } else {
            (
                format!("Receipt – {product_name}"),
                format!("<p>Attached is your receipt for <strong>{product_name}</strong>.</p>"),
            )
        };
        email_client
            .send_html_with_pdf(
                "Teknologappen",
                [email],
                &subject,
                body,
                format!("kvitto-{purchase_id}.pdf"),
                pdf,
            )
            .await
            .wrap_err_internal("failed to email receipt")?;
        Ok(())
    }
}

/// Loads the receipt fields of a purchase. Callers must authorize the purchase themselves.
///
/// # Errors
///
/// `DB`, `NOT_FOUND`, `ENCRYPTION`.
pub(crate) async fn load_receipt_target(
    ctx: &ContextWrapper,
    purchase_id: Uuid,
) -> MinilithResult<PurchaseReceiptTarget> {
    let row = sqlx::query!(
        r#"select
            purchases.product_id,
            products.group_id,
            products.name as "product_name!: DIS",
            purchases.activity_id,
            owner.name as "owner_name?",
            owner.language as "owner_language?"
        from purchases
        inner join products on products.id = purchases.product_id
        left join users owner on owner.id = purchases.owner_id
        where purchases.id = $1"#,
        purchase_id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;
    let decrypt = |value: Option<Vec<u8>>, field: &str| {
        value.map_or_else(
            || Ok(String::new()),
            |value| ctx.decrypt_string(value).wrap_err_encryption(field),
        )
    };
    Ok(PurchaseReceiptTarget {
        product_id: row.product_id,
        group_id: row.group_id,
        activity_id: row.activity_id,
        product_name: row.product_name.0,
        owner_name: decrypt(row.owner_name, "receipt: owner name")?,
        owner_language: decrypt(row.owner_language, "receipt: owner language")?,
    })
}

/// Who may see a purchase's receipt: admins of its product's group or of its activity.
async fn ensure_purchase_adminship(
    db: impl sqlx::PgExecutor<'_> + Copy,
    user_id: &str,
    target: &PurchaseReceiptTarget,
) -> MinilithResult<()> {
    let allowed = sqlx::query_scalar!(
        r#"select exists (
            select 1 from group_adminships
            where user_id = $1 and group_id = $2
        ) or ($3::uuid is not null and exists (
            select 1 from activity_hosts host
            inner join group_adminships admin on admin.group_id = host.group_id
            where host.activity_id = $3 and admin.user_id = $1
        )) as "exists!""#,
        user_id,
        target.group_id,
        target.activity_id,
    )
    .fetch_one(db)
    .await?;
    if allowed {
        Ok(())
    } else {
        Err(MinilithEndpointError::bad_frontend_code(
            "must administer the purchase's group or activity",
            "",
        ))
    }
}

/// Where to send a receipt. Both the admin API and sales devices use it.
#[derive(Object)]
pub(crate) struct ReceiptEmailRequest {
    /// Where to send the receipt.
    pub email: String,
}

/// One ownership step of a purchase: who got it, from whom, and what money changed hands.
#[derive(Debug, Clone, Object)]
pub struct PurchaseHistoryStep {
    pub buyer_id: String,
    pub buyer_name: String,
    /// The group for the first sale.
    pub seller_id: Option<String>,
    pub seller_name: String,
    pub completed_at: OffsetDateTime,
    /// `null` when the ticket was transferred instead of sold.
    pub transaction_id: Option<Uuid>,
    /// What the buyer paid in this step (the transaction's total, in ören). `null` for transfers.
    pub paid: Option<i64>,
    /// What was refunded to this buyer (in ören), if their payment was refunded.
    pub refunded: Option<i64>,
}

/// The ownership chain of a purchase, oldest first. A resale appends a step and refunds the
/// previous owner's payment; an admin refund leaves the chain unchanged but refunds the current
/// owner's payment, so both show up as `refunded` on the step they refunded.
///
/// Callers must authorize the purchase themselves.
///
/// # Errors
///
/// `DB`, `NOT_FOUND`, `ENCRYPTION`, or an internal error when the transactions service cannot
/// report the payments.
pub(crate) async fn load_purchase_history(
    ctx: &ContextWrapper,
    purchase_id: Uuid,
) -> MinilithResult<Vec<PurchaseHistoryStep>> {
    let purchase = sqlx::query!(
        r#"select refunded_id, paid_price as "paid_price!"
        from purchases where id = $1"#,
        purchase_id,
    )
    .fetch_optional(&ctx.db)
    .await?
    .wrap_err_not_found()?;
    let rows = sqlx::query!(
        r#"select
            transaction.buyer_id,
            buyer.name as "buyer_name?",
            transaction.seller_id,
            seller.name as "seller_name?",
            transaction.completed_at,
            transaction.transaction_id
        from purchase_transactions transaction
        left join users buyer on buyer.id = transaction.buyer_id
        left join users seller on seller.id = transaction.seller_id
        where transaction.purchase_id = $1
        order by transaction.completed_at, transaction.buyer_id"#,
        purchase_id,
    )
    .fetch_all(&ctx.db)
    .await?;
    let transaction_ids = rows
        .iter()
        .filter_map(|row| row.transaction_id)
        .collect::<Vec<_>>();
    let info = if transaction_ids.is_empty() {
        HashMap::new()
    } else {
        ctx.transactions_post("/v0/info")
            .json(&transactions::InfoRequest { transaction_ids })
            .send()
            .await
            .wrap_err_internal("failed to read the purchase transactions")?
            .error_for_status()
            .wrap_err_internal("failed to read the purchase transactions")?
            .json::<Vec<transactions::SingleInfoResponse>>()
            .await
            .wrap_err_internal("failed to parse the purchase transactions")?
            .into_iter()
            .map(|info| (info.id, info))
            .collect::<HashMap<_, _>>()
    };
    let decrypt = |value: Option<Vec<u8>>, field: &str| {
        value.map_or_else(
            || Ok(String::new()),
            |value| ctx.decrypt_string(value).wrap_err_encryption(field),
        )
    };
    let mut steps = Vec::with_capacity(rows.len());
    for row in rows {
        let payment = row.transaction_id.and_then(|id| info.get(&id));
        let paid = payment.map(|payment| payment.total);
        // A resale refunds the previous owner's payment (the provider was told the listed price,
        // which is that step's total), an admin refund the current owner's whole payment.
        let refunded = if row.transaction_id.is_some() && row.transaction_id == purchase.refunded_id
        {
            Some(purchase.paid_price.0)
        } else {
            payment
                .filter(|payment| matches!(payment.state, TransactionState::Refunded))
                .map(|payment| payment.total)
        };
        steps.push(PurchaseHistoryStep {
            buyer_id: row.buyer_id,
            buyer_name: decrypt(row.buyer_name, "purchase history: buyer name")?,
            seller_id: row.seller_id,
            seller_name: decrypt(row.seller_name, "purchase history: seller name")?,
            completed_at: row.completed_at,
            transaction_id: row.transaction_id,
            paid,
            refunded,
        });
    }
    Ok(steps)
}

#[OpenApi(prefix_path = "/v0/admin")]
impl Router {
    /// Lists purchases of a group's products or of one activity, newest first.
    ///
    /// Exactly one of `group_id` (direct adminship) and `activity_id` (activity adminship) is
    /// required. `start`/`end` filter on the purchase time, `end` exclusive.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `BAD_REQUEST` (both or neither scope given).
    #[oai(path = "/purchases", method = "get")]
    async fn purchases(
        &self,
        user: User,
        Query(group_id): Query<Option<Uuid>>,
        Query(activity_id): Query<Option<Uuid>>,
        Query(start): Query<Option<OffsetDateTime>>,
        Query(end): Query<Option<OffsetDateTime>>,
    ) -> MinilithResult<Json<Vec<PurchaseRow>>> {
        let (group_id, activity_id) = match (group_id, activity_id) {
            (Some(group_id), None) => {
                check_direct_adminship(&self.db, user.get_id(), group_id).await?;
                (Some(group_id), None)
            }
            (None, Some(activity_id)) => {
                check_activity_adminship(&self.db, user.get_id(), activity_id).await?;
                (None, Some(activity_id))
            }
            _ => {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "exactly one of group_id and activity_id is required",
                    "",
                ));
            }
        };
        load_purchases(
            self,
            PurchaseQuery {
                product_ids: None,
                group_id,
                activity_id,
                start,
                end,
            },
        )
        .await
        .map(Json)
    }

    /// Products sold in an interval, for a group's products or one activity.
    ///
    /// Always lists every product in scope, also those with no sales in the interval.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `BAD_REQUEST` (both or neither scope given).
    #[oai(path = "/purchases/sold", method = "get")]
    async fn sold_products(
        &self,
        user: User,
        Query(group_id): Query<Option<Uuid>>,
        Query(activity_id): Query<Option<Uuid>>,
        Query(start): Query<Option<OffsetDateTime>>,
        Query(end): Query<Option<OffsetDateTime>>,
    ) -> MinilithResult<Json<Vec<SoldProduct>>> {
        let (group_id, activity_id) = match (group_id, activity_id) {
            (Some(group_id), None) => {
                check_direct_adminship(&self.db, user.get_id(), group_id).await?;
                (Some(group_id), None)
            }
            (None, Some(activity_id)) => {
                check_activity_adminship(&self.db, user.get_id(), activity_id).await?;
                (None, Some(activity_id))
            }
            _ => {
                return Err(MinilithEndpointError::bad_frontend_code(
                    "exactly one of group_id and activity_id is required",
                    "",
                ));
            }
        };
        let rows = sqlx::query!(
            r#"select
                products.id,
                products.name as "name!: DIS",
                products.kind::text as "kind!",
                products.max_uses,
                count(purchases.id) as "purchases!",
                count(purchases.id) filter (where purchases.refunded_id is null) as "sold!",
                count(purchases.id) filter (where purchases.refunded_id is not null)
                    as "refunded!",
                coalesce(sum(purchases.paid_price)
                    filter (where purchases.refunded_id is null), '0.00'::money) as "revenue!",
                coalesce(sum((select count(*) from purchase_uses uses
                    where uses.purchase_id = purchases.id)), 0)::bigint as "uses!"
            from products
            left join purchases on purchases.product_id = products.id
                and ($2::uuid is null or purchases.activity_id = $2)
                and ($3::timestamptz is null or purchases.purchased_at >= $3)
                and ($4::timestamptz is null or purchases.purchased_at < $4)
            where ($1::uuid is null or products.group_id = $1)
            and ($2::uuid is null or exists (
                select 1 from product_activity_assignments assignment
                where assignment.product_id = products.id
                and assignment.activity_id = $2
            ))
            and products.kind <> 'activity_visibility'
            group by products.id
            order by products.name->>'en'"#,
            group_id,
            activity_id,
            start,
            end,
        )
        .fetch_all(&self.db)
        .await?;
        let mut products = Vec::with_capacity(rows.len());
        for row in rows {
            products.push(SoldProduct {
                product_id: row.id,
                name: row.name.0,
                kind: AdminProductKind::from_db(&row.kind)?,
                max_uses: row.max_uses,
                sold: row.sold,
                refunded: row.refunded,
                revenue: row.revenue.0,
                uses: row.uses,
                uses_possible: i64::from(row.max_uses) * row.sold,
            });
        }
        Ok(Json(products))
    }

    /// The ownership chain of a purchase: who bought or received it, what they paid and whether
    /// that payment was refunded. Same authorization as the receipt.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `NOT_FOUND`.
    #[oai(path = "/purchases/:id/history", method = "get")]
    async fn purchase_history(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<PurchaseHistoryStep>>> {
        let target = load_receipt_target(self, id).await?;
        ensure_purchase_adminship(&self.db, user.get_id(), &target).await?;
        load_purchase_history(self, id).await.map(Json)
    }

    /// Downloads the current owner's receipt for a purchase as a PDF.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `NOT_FOUND`, `BAD_REQUEST` (transferred purchase, no payment).
    #[oai(path = "/purchases/:id/receipt", method = "get")]
    async fn purchase_receipt(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Response<Binary<poem::Body>>> {
        let target = load_receipt_target(self, id).await?;
        ensure_purchase_adminship(&self.db, user.get_id(), &target).await?;
        let pdf = target.receipt(self, id).await?;
        Ok(Response::new(Binary(pdf.into())).header("content-type", "application/octet-stream"))
    }

    /// Emails the current owner's receipt for a purchase to an address of the admin's choice.
    ///
    /// # Errors
    ///
    /// `AUTH`, `DB`, `NOT_FOUND`, `BAD_REQUEST` (transferred purchase, no payment, bad address,
    /// email not configured).
    #[oai(path = "/purchases/:id/receipt", method = "post")]
    async fn email_purchase_receipt(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<ReceiptEmailRequest>,
    ) -> MinilithResult<()> {
        let target = load_receipt_target(self, id).await?;
        ensure_purchase_adminship(&self.db, user.get_id(), &target).await?;
        target.email_receipt(self, id, &body.email).await
    }
}
