//! Product-first application (end-user) API.
//!
//! Everything here is expressed in terms of products and purchases, mirroring
//! the admin product API and the database. The deprecated ticket API in
//! [`super::legacy`] stays as a thin transport shim over the same operations.

#![allow(
    clippy::module_name_repetitions,
    reason = "`App` prefixes make the public product DTOs unambiguous"
)]

use fed_auth_verifier::User;
use minilith_errors::MinilithErrorOptionExt as _;
use poem_openapi::param::{Path, Query};
use poem_openapi::payload::{Binary, Json, Response};
use poem_openapi::{Enum, Object, OpenApi};
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use super::models::{
    BoughtAddon, BuyTicketRequest, BuyTicketResponse, PurchaseProvider, PurchaseStatus,
    PurchasedAddon, QueueRequest, QueueResponse,
};
use super::validation::{ValidateResponse, ValidateUseRequest};
use super::{catalog, purchase, queue, sales, transfer, validation};
use crate::push_notifications::{NotificationKind, load_notification_history};
use crate::readers;
use crate::{
    DbInternationalizedString as DIS, InternationalizedString as IS, MinilithEndpointError,
    MinilithResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum, sqlx::Type)]
#[oai(rename_all = "snake_case")]
#[sqlx(type_name = "product_kind", rename_all = "snake_case")]
pub enum ProductKind {
    Ticket,
    Normal,
    Questionnaire,
    ActivityVisibility,
}

#[derive(Debug, Object)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the store and ticket switches are independent"
)]
pub struct AppProduct {
    pub id: Uuid,
    pub group_id: Uuid,
    pub group_path: String,
    pub group_name: IS,
    pub kind: ProductKind,
    /// Whether the caller owns at least one un-refunded copy.
    pub purchased_by_me: bool,
    pub name: IS,
    pub description: IS,
    pub image_url: Option<String>,
    /// The product's own (base) price.
    pub price_min: i64,
    /// Base price plus the most expensive option of every add-on.
    pub price_max: i64,
    pub max_uses: i32,
    /// `None` when the stock is not managed automatically (physical sales only).
    pub sales_stock: Option<i32>,
    pub sales_start: OffsetDateTime,
    pub sales_stop: OffsetDateTime,
    pub resale_start: OffsetDateTime,
    pub resale_stop: OffsetDateTime,
    pub tickets_been_released: bool,
    pub store_general_sales: bool,
    pub store_online_self_serve: bool,
    pub activity_ids: Vec<Uuid>,
}

#[derive(Debug, Object)]
pub struct AppProductDetails {
    #[oai(flatten)]
    pub product: AppProduct,
    pub available_addons: Vec<super::models::AvailableAddon>,
}

#[derive(Debug, Object)]
pub struct AppPurchase {
    pub id: Uuid,
    pub product_id: Uuid,
    pub product_name: IS,
    pub kind: ProductKind,
    /// Price actually paid (may differ from the product's current price).
    pub paid_price: i64,
    pub purchased_at: OffsetDateTime,
    pub activity_id: Option<Uuid>,
    pub activity_title: Option<IS>,
    pub activity_time_start: Option<OffsetDateTime>,
    pub activity_time_end: Option<OffsetDateTime>,
    pub group_id: Uuid,
    pub group_path: String,
    pub group_name: IS,
    pub image_url: Option<String>,
    /// False if the caller transferred it away.
    pub owned_by_me: bool,
    /// A purchase with any use can no longer be transferred or resold.
    pub uses_left: i32,
    pub max_uses: i32,
    /// The latest use; its id is what the QR code encodes. `None` until the first use.
    pub last_use_id: Option<Uuid>,
    /// Non-null when the owner put the purchase up for resale.
    pub for_resale: Option<i64>,
    pub refunded: bool,
    /// Whether the caller can transfer it right now (recipient rules are per-recipient).
    pub transferable: bool,
    pub purchased_addons: Vec<PurchasedAddon>,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct AppResale {
    pub purchase_id: Uuid,
    pub price: i64,
    pub purchased_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct AppQueueRequest {
    pub product_id: Uuid,
}

#[derive(Debug, Object)]
pub struct AppBoughtAddon {
    pub addon_id: Uuid,
    pub selected_options: Vec<i32>,
    pub selected_text: Option<String>,
}

#[derive(Debug, Object)]
pub struct AppPurchaseRequest {
    pub provider: PurchaseProvider,
    pub addons: Vec<AppBoughtAddon>,
    /// Required for `SumUp`; where the hosted checkout returns.
    pub redirect_url: Option<String>,
    /// Pay on this `SumUp` reader instead of a hosted checkout (skips the reservation flow).
    pub reader_id: Option<String>,
}

#[derive(Debug, Object)]
pub struct AppSaleItem {
    pub product_id: Uuid,
    pub addons: Vec<AppBoughtAddon>,
}

#[derive(Debug, Object)]
pub struct AppSaleRequest {
    pub items: Vec<AppSaleItem>,
    /// Defaults to the anonymous counter-sale user.
    pub buyer_id: Option<String>,
    pub reader_id: String,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct AppSaleStarted {
    pub transaction_id: Uuid,
}

#[derive(Debug, Object)]
pub struct AppTransferRequest {
    pub purchase_id: Uuid,
    pub to_user: String,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct AppUseResponse {
    /// The newly created use. Encode this in the QR code the verifier scans.
    pub use_id: Uuid,
    pub uses_left: i32,
}

#[derive(Debug, Clone, Object)]
pub struct AppResalePurchaseRequest {
    pub provider: PurchaseProvider,
    /// Required for `SumUp`; where the hosted checkout returns.
    pub redirect_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct AppNoShowHint {
    pub no_show_hint: bool,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct AppResaleRequest {
    /// The asking price; cannot exceed what the owner paid.
    pub price: i64,
}

#[derive(Debug, Object)]
pub struct AppActivityNotification {
    pub id: Uuid,
    pub title: IS,
    pub content: IS,
    pub send_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Object)]
pub struct AppNotificationEvent {
    pub notification_id: Uuid,
    pub kind: NotificationEventKind,
    /// E.g. the activity or news the notification pointed to.
    pub target_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, Enum, sqlx::Type)]
#[oai(rename_all = "snake_case")]
#[sqlx(type_name = "notification_event_kind", rename_all = "snake_case")]
pub enum NotificationEventKind {
    Sent,
    Opened,
    Dismissed,
}

#[derive(Debug, Object)]
pub struct AppNotification {
    pub id: Uuid,
    pub kind: NotificationKind,
    /// Activity id, news id or product id depending on `kind`.
    pub target_id: Option<Uuid>,
    pub sender: IS,
    pub title: IS,
    pub content: IS,
    pub send_at: OffsetDateTime,
    pub read: bool,
}

#[derive(Debug, Clone, Object)]
pub struct AppMarkNotificationsRead {
    pub ids: Vec<Uuid>,
}

/// Who is looking at products: an app user or a registered sales device.
#[derive(Debug, Clone, Copy)]
enum ProductAccess<'a> {
    User(&'a str),
    Device(sales::DeviceScope),
}

impl<'a> ProductAccess<'a> {
    fn user_id(self) -> &'a str {
        match self {
            ProductAccess::User(user_id) => user_id,
            ProductAccess::Device(_) => "",
        }
    }
    fn device_group(self) -> Option<Uuid> {
        match self {
            ProductAccess::User(_) => None,
            ProductAccess::Device(device) => device.group_id,
        }
    }
    fn device_activity(self) -> Option<Uuid> {
        match self {
            ProductAccess::User(_) => None,
            ProductAccess::Device(device) => device.activity_id,
        }
    }
}

/// The single product query used by both the app and sales devices. A device only sees products
/// it is registered to sell (`store_general_sales` within its group/activity).
#[allow(
    clippy::too_many_lines,
    reason = "one query expresses the whole product list with its access rules"
)]
async fn load_products(
    ctx: &crate::context::ContextWrapper,
    access: ProductAccess<'_>,
    activity_id: Option<Uuid>,
    group_id: Option<Uuid>,
    product_id: Option<Uuid>,
) -> MinilithResult<Vec<AppProduct>> {
    let products = sqlx::query!(
        r#"select
            product.id,
            product.group_id,
            owner.path::text as "group_path!",
            owner.name as "group_name!: DIS",
            product.kind as "kind!: ProductKind",
            product.name as "name!: DIS",
            product.description as "description!: DIS",
            image.url as "image_url?",
            (select range.price_min from product_price_ranges range
                where range.product_id = product.id) as "price_min!",
            (select range.price_max from product_price_ranges range
                where range.product_id = product.id) as "price_max!",
            product.max_uses,
            product.sales_stock,
            product.sales_start,
            product.sales_stop,
            product.resale_start,
            product.resale_stop,
            product.tickets_been_released,
            product.store_general_sales,
            product.store_online_self_serve,
            coalesce(
                (select array_agg(assignment.activity_id)
                    from product_activity_assignments assignment
                    where assignment.product_id = product.id),
                '{}'::uuid[]
            ) as "activity_ids!: Vec<Uuid>",
            exists (
                select 1 from purchases owned
                where owned.product_id = product.id
                and owned.owner_id = $1
                and owned.refunded_id is null
            ) as "purchased_by_me!"
        from products product
        inner join groups owner on owner.id = product.group_id
        left join images image on image.id = product.image_id
        where product.kind <> 'activity_visibility'
        and ($2::uuid is null or exists (
            select 1 from product_activity_assignments assignment
            where assignment.product_id = product.id
            and assignment.activity_id = $2
        ))
        and ($3::uuid is null or product.group_id = $3)
        and ($6::uuid is null or product.id = $6)
        and (
            -- a registered sales device sells what it is registered for
            (($4::uuid is not null or $5::uuid is not null)
                and product.store_general_sales
                and (
                    ($4::uuid is not null and (
                        product.group_id = $4
                        or exists (
                            select 1
                            from product_activity_assignments assignment
                            inner join activity_hosts host
                                on host.activity_id = assignment.activity_id
                            where assignment.product_id = product.id
                            and host.group_id = $4
                        )
                    ))
                    or ($5::uuid is not null and exists (
                        select 1 from product_activity_assignments assignment
                        where assignment.product_id = product.id
                        and assignment.activity_id = $5
                    ))
                ))
            -- app visibility
            or (($4::uuid is null and $5::uuid is null) and (
                ($2::uuid is null and $3::uuid is null
                    and product.store_online_self_serve)
                or exists (
                    select 1
                    from product_allowed_groups allowed
                    inner join group_memberships membership
                        on membership.user_id = $1
                    inner join groups member_group
                        on member_group.id = membership.group_id
                    inner join groups allowed_group
                        on allowed_group.id = allowed.group_id
                        and allowed_group.path @> member_group.path
                    where allowed.product_id = product.id
                    and (
                        not membership.specific
                        or allowed.group_id = membership.group_id
                    )
                )
                or exists (
                    select 1 from product_buyers buyer
                    where buyer.product_id = product.id and buyer.user_id = $1
                )
                or exists (
                    select 1 from purchases owned
                    where owned.product_id = product.id
                    and owned.owner_id = $1
                )
            ))
        )
        order by product.sales_start, product.name->>'en'"#,
        access.user_id(),
        activity_id,
        group_id,
        access.device_group(),
        access.device_activity(),
        product_id,
    )
    .map(|row| AppProduct {
        id: row.id,
        group_id: row.group_id,
        group_path: row.group_path,
        group_name: row.group_name.0,
        kind: row.kind,
        purchased_by_me: row.purchased_by_me,
        name: row.name.0,
        description: row.description.0,
        image_url: row.image_url,
        price_min: row.price_min.0,
        price_max: row.price_max.0,
        max_uses: row.max_uses,
        sales_stock: row.sales_stock,
        sales_start: row.sales_start,
        sales_stop: row.sales_stop,
        resale_start: row.resale_start,
        resale_stop: row.resale_stop,
        tickets_been_released: row.tickets_been_released,
        store_general_sales: row.store_general_sales,
        store_online_self_serve: row.store_online_self_serve,
        activity_ids: row.activity_ids,
    })
    .fetch_all(&ctx.db)
    .await?;
    Ok(products)
}

async fn load_product(
    ctx: &crate::context::ContextWrapper,
    access: ProductAccess<'_>,
    id: Uuid,
) -> MinilithResult<AppProductDetails> {
    let product = load_products(ctx, access, None, None, Some(id))
        .await?
        .into_iter()
        .next()
        .wrap_err_not_found()?;
    let activity_id = access
        .device_activity()
        .or(product.activity_ids.first().copied());
    let mut txn = ctx.db.begin().await?;
    let available_addons = super::load_purchasable_addons(&mut txn, id, activity_id).await?;
    Ok(AppProductDetails {
        product,
        available_addons,
    })
}

/// Reads and validates the `x-sales-device-id` header. The UUID is the device's credential.
async fn sales_device_from_request(
    ctx: &crate::context::ContextWrapper,
    req: &poem::Request,
) -> MinilithResult<sales::DeviceScope> {
    let id = req
        .headers()
        .get("x-sales-device-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| {
            MinilithEndpointError::unauthorized("missing or invalid x-sales-device-id", "")
        })?;
    sales::load_device(&ctx.db, id).await
}

/// The ids of the products a registered sales device may sell.
async fn device_product_ids(
    ctx: &crate::context::ContextWrapper,
    device: sales::DeviceScope,
) -> MinilithResult<Vec<Uuid>> {
    Ok(
        load_products(ctx, ProductAccess::Device(device), None, None, None)
            .await?
            .into_iter()
            .map(|product| product.id)
            .collect(),
    )
}

impl AppRouter {
    /// A purchase's receipt fields, if the purchase is of a product this device may sell.
    async fn device_receipt_target(
        &self,
        device: sales::DeviceScope,
        purchase_id: Uuid,
    ) -> MinilithResult<crate::admin_purchases::PurchaseReceiptTarget> {
        let target = crate::admin_purchases::load_receipt_target(self, purchase_id).await?;
        let allowed = device_product_ids(self, device).await?;
        if !allowed.contains(&target.product_id) {
            return Err(MinilithEndpointError::not_found());
        }
        Ok(target)
    }
}

#[derive(Clone, Debug)]
pub struct AppRouter {
    pub context: crate::context::ContextWrapper,
}

impl std::ops::Deref for AppRouter {
    type Target = crate::context::ContextWrapper;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

#[OpenApi(prefix_path = "/v1")]
impl AppRouter {
    /// Readers that may be used to pay for this activity or group.
    ///
    /// Pass exactly one of `activity_id` and `group_id`. Activity readers come from the activity's
    /// hosts, its sales-target group and its stock group.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/readers", method = "get")]
    async fn readers(
        &self,
        user: User,
        Query(activity_id): Query<Option<Uuid>>,
        Query(group_id): Query<Option<Uuid>>,
    ) -> MinilithResult<Json<Vec<readers::GroupReader>>> {
        match (activity_id, group_id) {
            (Some(activity_id), None) => {
                self.test_activity_access(user.get_id(), &activity_id)
                    .await?;
                let group_ids = sqlx::query_scalar!(
                    r#"select group_id from activity_hosts where activity_id = $1
                    union
                    select target.group_id
                        from activities
                        inner join sales_targets target on target.id = activities.sales_target_id
                        where activities.id = $1
                    union
                    select stock_group from activities
                        where id = $1 and stock_group is not null"#,
                    activity_id,
                )
                .fetch_all(&self.db)
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
                Ok(Json(
                    readers::for_activity(&self.db, self, &group_ids).await?,
                ))
            }
            (None, Some(group_id)) => {
                let allowed = sqlx::query_scalar!(
                    r#"select exists (
                        select 1 from group_memberships where user_id = $1 and group_id = $2
                    ) or exists (
                        select 1 from group_adminships where user_id = $1 and group_id = $2
                    ) as "exists!""#,
                    user.get_id(),
                    group_id,
                )
                .fetch_one(&self.db)
                .await?;
                if !allowed {
                    return Err(MinilithEndpointError::bad_frontend_code(
                        "must be a member of the group",
                        "",
                    ));
                }
                Ok(Json(readers::for_group(&self.db, self, group_id).await?))
            }
            _ => Err(MinilithEndpointError::bad_frontend_code(
                "exactly one of activity_id and group_id is required",
                "",
            )),
        }
    }

    /// Products a registered sales device may sell.
    ///
    /// The device is identified by the `x-sales-device-id` header (the UUID in its QR code).
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/sales-device/products", method = "get")]
    async fn sales_device_products(
        &self,
        req: &poem::Request,
    ) -> MinilithResult<Json<Vec<AppProduct>>> {
        let device = sales_device_from_request(self, req).await?;
        load_products(self, ProductAccess::Device(device), None, None, None)
            .await
            .map(Json)
    }

    /// A product with the add-ons a sales device can select when selling it.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/sales-device/products/:id", method = "get")]
    async fn sales_device_product(
        &self,
        req: &poem::Request,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<AppProductDetails>> {
        let device = sales_device_from_request(self, req).await?;
        load_product(self, ProductAccess::Device(device), id)
            .await
            .map(Json)
    }

    /// Card readers the sales device may charge on.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/sales-device/readers", method = "get")]
    async fn sales_device_readers(
        &self,
        req: &poem::Request,
    ) -> MinilithResult<Json<Vec<readers::GroupReader>>> {
        let device = sales_device_from_request(self, req).await?;
        let mut txn = self.db.begin().await?;
        let group_ids = sales::reader_group_ids(&mut txn, &device).await?;
        txn.commit().await?;
        let readers = if device.activity_id.is_some() {
            readers::for_activity(&self.db, self, &group_ids).await?
        } else {
            readers::for_group(
                &self.db,
                self,
                device.group_id.wrap_err_internal("device has no scope")?,
            )
            .await?
        };
        Ok(Json(readers))
    }

    /// Starts one counter sale for all items and pays it on a reader. The sale completes when the
    /// provider callback arrives; poll `GET /v1/sales-device/sales/{transaction_id}`.
    ///
    /// # Errors
    ///
    /// AUTH, DB, or the provider rejecting the payment.
    #[oai(path = "/sales-device/purchases", method = "post")]
    async fn sales_device_purchase(
        &self,
        req: &poem::Request,
        Json(body): Json<AppSaleRequest>,
    ) -> MinilithResult<Json<AppSaleStarted>> {
        let device = sales_device_from_request(self, req).await?;
        let buyer_id = body
            .buyer_id
            .unwrap_or_else(|| sales::ANONYMOUS_BUYER.to_owned());
        let items = body
            .items
            .into_iter()
            .map(|item| sales::Item {
                product_id: item.product_id,
                addons: item
                    .addons
                    .into_iter()
                    .map(|addon| BoughtAddon {
                        id: addon.addon_id,
                        selected_options: Some(addon.selected_options),
                        selected_text: addon.selected_text,
                    })
                    .collect(),
            })
            .collect();
        let transaction_id = sales::begin(
            self,
            sales::Viewer::Device(&device),
            &buyer_id,
            items,
            Some(&body.reader_id),
        )
        .await?;
        Ok(Json(AppSaleStarted { transaction_id }))
    }

    /// The state and resulting purchases of a counter sale this device started.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/sales-device/sales/:transaction_id", method = "get")]
    async fn sales_device_sale(
        &self,
        req: &poem::Request,
        Path(transaction_id): Path<Uuid>,
    ) -> MinilithResult<Json<sales::SaleStatus>> {
        let device = sales_device_from_request(self, req).await?;
        sales::status(self, &device, transaction_id).await.map(Json)
    }

    /// Purchases of the products this device may sell, newest first.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/sales-device/purchases", method = "get")]
    async fn sales_device_purchases(
        &self,
        req: &poem::Request,
        Query(start): Query<Option<OffsetDateTime>>,
        Query(end): Query<Option<OffsetDateTime>>,
    ) -> MinilithResult<Json<Vec<crate::admin_purchases::PurchaseRow>>> {
        let device = sales_device_from_request(self, req).await?;
        crate::admin_purchases::load_purchases(
            self,
            crate::admin_purchases::PurchaseQuery {
                product_ids: Some(device_product_ids(self, device).await?),
                group_id: None,
                activity_id: None,
                start,
                end,
            },
        )
        .await
        .map(Json)
    }

    /// Downloads the current owner's receipt for a purchase of a product this device may sell.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND, BAD REQUEST (transferred purchase, no payment).
    #[oai(path = "/sales-device/purchases/:id/receipt", method = "get")]
    async fn sales_device_receipt(
        &self,
        req: &poem::Request,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Response<Binary<poem::Body>>> {
        let device = sales_device_from_request(self, req).await?;
        let target = self.device_receipt_target(device, id).await?;
        let pdf = target.receipt(self, id).await?;
        Ok(Response::new(Binary(pdf.into())).header("content-type", "application/octet-stream"))
    }

    /// Emails the current owner's receipt for a purchase to an address of the caller's choice.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND, BAD REQUEST (transferred purchase, no payment, bad address, email
    /// not configured).
    #[oai(path = "/sales-device/purchases/:id/receipt", method = "post")]
    async fn sales_device_email_receipt(
        &self,
        req: &poem::Request,
        Path(id): Path<Uuid>,
        Json(body): Json<crate::admin_purchases::ReceiptEmailRequest>,
    ) -> MinilithResult<()> {
        let device = sales_device_from_request(self, req).await?;
        let target = self.device_receipt_target(device, id).await?;
        target.email_receipt(self, id, &body.email).await
    }

    /// Products the caller may buy or already owns.
    ///
    /// Filter by `activity_id` (products assigned to that activity) or
    /// `group_id` (products owned by that group). Without a filter this
    /// returns self-serve store products the caller has access to.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[allow(
        clippy::too_many_lines,
        reason = "one query expresses the whole product list with its access rules"
    )]
    #[oai(path = "/products", method = "get")]
    #[allow(clippy::too_many_lines, reason = "linear product query and mapping")]
    async fn products(
        &self,
        user: User,
        activity_id: Query<Option<Uuid>>,
        group_id: Query<Option<Uuid>>,
    ) -> MinilithResult<Json<Vec<AppProduct>>> {
        load_products(
            self,
            ProductAccess::User(user.get_id()),
            activity_id.0,
            group_id.0,
            None,
        )
        .await
        .map(Json)
    }

    /// A product with the add-ons that can be selected when buying it.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/products/:id", method = "get")]
    async fn product(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<AppProductDetails>> {
        load_product(self, ProductAccess::User(user.get_id()), id)
            .await
            .map(Json)
    }

    /// All purchases the caller bought or owns, including transferred ones.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/purchases", method = "get")]
    #[allow(clippy::too_many_lines, reason = "linear product and add-on mapping")]
    async fn purchases(&self, user: User) -> MinilithResult<Json<Vec<AppPurchase>>> {
        let id = user.get_id();
        let mut addons: std::collections::HashMap<Uuid, Vec<PurchasedAddon>> = sqlx::query!(
            r#"select
                purchased_product_addons.purchase_id,
                product_addons.id as addon_id,
                product_addons.name as "addon_name!: DIS",
                product_addons.multiple_alternatives,
                product_addons.has_text_field,
                product_addons.required,
                purchased_product_addons.selected_options,
                purchased_product_addons.selected_text
            from purchases
            inner join purchased_product_addons
                on purchased_product_addons.purchase_id = purchases.id
            inner join product_addons
                on product_addons.id = purchased_product_addons.addon_id
            where purchases.owner_id = $1 or purchases.purchaser_id = $1
            order by product_addons.idx"#,
            id
        )
        .map(|row| {
            (
                row.purchase_id,
                PurchasedAddon {
                    inner: super::models::Addon {
                        id: row.addon_id,
                        name: row.addon_name.0,
                        multiple_alternatives: row.multiple_alternatives,
                        has_text_field: row.has_text_field,
                        required: row.required,
                    },
                    selected_options: row.selected_options,
                    selected_text: row.selected_text,
                    options: Vec::new(),
                },
            )
        })
        .fetch_all(&self.db)
        .await?
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut map, (id, addon)| {
            map.entry(id).or_default().push(addon);
            map
        });

        let now = OffsetDateTime::now_utc();
        let purchases = sqlx::query!(
            r#"select
                purchase.id,
                product.id as product_id,
                product.name as "product_name!: DIS",
                product.kind as "kind!: ProductKind",
                purchase.paid_price,
                purchase.purchased_at,
                purchase.activity_id,
                activity.title as "activity_title?: DIS",
                activity.time_start as "activity_time_start?",
                activity.time_end as "activity_time_end?",
                product.group_id,
                owner_group.path::text as "group_path!",
                owner_group.name as "group_name!: DIS",
                image.url as "image_url?",
                (purchase.owner_id = $1) as "owned_by_me!",
                product.max_uses,
                (select count(*)::int from purchase_uses used
                    where used.purchase_id = purchase.id) as "uses_count!",
                case when purchase.owner_id = $1 then
                    (select used.id from purchase_uses used
                        where used.purchase_id = purchase.id
                        order by used.timestamp desc, used.id desc
                        limit 1)
                end as "last_use_id?",
                purchase.for_resale,
                product.resale_start,
                product.resale_stop,
                exists (select 1 from purchase_uses used
                    where used.purchase_id = purchase.id) as "used!",
                (purchase.refunded_id is not null) as "refunded!"
            from purchases purchase
            inner join products product on product.id = purchase.product_id
            inner join groups owner_group on owner_group.id = product.group_id
            left join images image on image.id = product.image_id
            left join activities activity on activity.id = purchase.activity_id
            where exists (
                select 1 from purchase_transactions history
                where history.purchase_id = purchase.id
                and history.buyer_id = $1
            )
            order by purchase.purchased_at desc"#,
            id
        )
        .map(|row| AppPurchase {
            id: row.id,
            product_id: row.product_id,
            product_name: row.product_name.0,
            kind: row.kind,
            paid_price: row.paid_price.0,
            purchased_at: row.purchased_at,
            activity_id: row.activity_id,
            activity_title: row.activity_title.map(|title| title.0),
            activity_time_start: row.activity_time_start,
            activity_time_end: row.activity_time_end,
            group_id: row.group_id,
            group_path: row.group_path,
            group_name: row.group_name.0,
            image_url: row.image_url,
            owned_by_me: row.owned_by_me,
            uses_left: row.max_uses - row.uses_count,
            max_uses: row.max_uses,
            last_use_id: row.last_use_id,
            for_resale: row.for_resale.map(|price| price.0),
            refunded: row.refunded,
            transferable: transfer::is_transferable(
                row.owned_by_me,
                row.used,
                row.refunded,
                row.resale_start,
                row.resale_stop,
                now,
            ),
            purchased_addons: addons.remove(&row.id).unwrap_or_default(),
        })
        .fetch_all(&self.db)
        .await?;

        Ok(Json(purchases))
    }

    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/purchases/:id/receipt", method = "get")]
    async fn receipt(
        &self,
        auth: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Response<Binary<poem::Body>>> {
        catalog::receipt(self, auth, id).await
    }

    /// Start a use of a purchase (e.g. a clip card or a physical item).
    ///
    /// Every call inserts a new use and consumes one of `max_uses`. The returned
    /// `use_id` is the QR payload the verifier scans; `GET /purchases` exposes the
    /// latest one as `last_use_id` so the app can keep showing the same QR.
    ///
    /// **Using a purchase makes it non-transferable and non-resellable** (both
    /// operations reject purchases with any use, see `POST /purchases/:id/transfer`
    /// and `POST /purchases/:id/resale`). It also removes an existing resale listing.
    /// The app must only call this when the user explicitly wants to use the
    /// purchase, never just to display a ticket.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND. The purchase must be owned, unrefunded, have a use
    /// left and not be in the middle of a resale.
    #[oai(path = "/purchases/:id/use", method = "post")]
    async fn use_purchase(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<AppUseResponse>> {
        let mut txn = self.db.begin().await?;
        // Lock the purchase so concurrent `/use` calls cannot both pass the
        // `max_uses` check.
        let purchase = sqlx::query!(
            r#"select
                purchase.owner_id,
                (purchase.refunded_id is not null) as "refunded!",
                (purchase.for_resale is not null) as "for_resale!",
                (purchase.resale_transaction_id is not null) as "resale_in_progress!",
                product.max_uses,
                (select count(*)::int from purchase_uses used
                    where used.purchase_id = purchase.id) as "uses_count!"
            from purchases purchase
            inner join products product on product.id = purchase.product_id
            where purchase.id = $1
            for update of purchase"#,
            id,
        )
        .fetch_optional(&mut txn.executor())
        .await?
        .wrap_err_not_found()?;
        if purchase.owner_id != user.get_id() {
            return Err(MinilithEndpointError::bad_frontend_code(
                "you do not own this purchase",
                "",
            ));
        }
        if purchase.refunded {
            return Err(MinilithEndpointError::bad_frontend_code(
                "purchase has been refunded",
                "",
            ));
        }
        if purchase.resale_in_progress {
            return Err(MinilithEndpointError::bad_frontend_code(
                "purchase is being resold and cannot be used",
                "",
            ));
        }
        let uses_left = purchase.max_uses - purchase.uses_count;
        if uses_left <= 0 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "purchase has no uses left",
                "",
            ));
        }
        if purchase.for_resale {
            sqlx::query!("update purchases set for_resale = null where id = $1", id)
                .execute(&mut txn.executor())
                .await?;
        }
        let use_id = sqlx::query_scalar!(
            "insert into purchase_uses (id, purchase_id) values (uuidv4(), $1) returning id",
            id
        )
        .fetch_one(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(Json(AppUseResponse {
            use_id,
            uses_left: uses_left - 1,
        }))
    }

    /// Buy a purchase that is listed for resale.
    ///
    /// Charges the resale price plus the seller's refund fee and the new
    /// transaction fee. Add-ons stay exactly as they were. A used purchase is
    /// never offered: using one removes its listing and both listing and buying
    /// reject purchases with any use.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/purchases/:id/resale/purchase", method = "post")]
    async fn purchase_resale(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<AppResalePurchaseRequest>,
    ) -> MinilithResult<Json<BuyTicketResponse>> {
        purchase::begin_resale(self, user, id, body.provider, body.redirect_url)
            .await
            .map(Json)
    }

    /// Put an owned purchase up for resale.
    ///
    /// Only allowed when every transaction of the purchase is attached to a
    /// transaction id, otherwise a refund cannot be guaranteed to reach the
    /// right person. A used purchase cannot be listed.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/purchases/:id/resale", method = "post")]
    async fn mark_for_resale(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<AppResaleRequest>,
    ) -> MinilithResult<()> {
        super::ensure_price_is_allowed(body.price)?;
        let mut txn = self.db.begin().await?;
        // Serialize with `POST /purchases/{id}/use` (which locks the same row) so a
        // use that commits concurrently cannot slip past the "unused" check below.
        sqlx::query_scalar!("select id from purchases where id = $1 for update", id)
            .fetch_optional(&mut txn.executor())
            .await?
            .wrap_err_not_found()?;
        let result = sqlx::query!(
            r#"update purchases purchase
            set for_resale = $3::money
            from products product
            where purchase.id = $1
            and purchase.product_id = product.id
            and purchase.owner_id = $2
            and purchase.refunded_id is null
            and purchase.for_resale is null
            and now() between product.resale_start and product.resale_stop
            and not exists (
                select 1 from purchase_transactions transaction
                where transaction.purchase_id = purchase.id
                and transaction.transaction_id is null
            )
            and not exists (
                select 1 from purchase_uses used
                where used.purchase_id = purchase.id
            )
            and $3::money <= purchase.paid_price"#,
            id,
            user.get_id(),
            sqlx::postgres::types::PgMoney(body.price),
        )
        .execute(&mut txn.executor())
        .await?;
        if result.rows_affected() == 0 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "purchase cannot be put up for resale right now",
                "",
            ));
        }
        txn.commit().await?;
        Ok(())
    }

    /// Purchases of this product that are currently offered for resale.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/products/:id/resale", method = "get")]
    async fn resale_offers(
        &self,
        _user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<AppResale>>> {
        let offers = sqlx::query!(
            r#"select purchase.id, purchase.for_resale as "price!", purchase.purchased_at
            from purchases purchase
            where purchase.product_id = $1
            and purchase.for_resale is not null
            and purchase.refunded_id is null
            order by purchase.for_resale, purchase.purchased_at"#,
            id
        )
        .map(|row| AppResale {
            purchase_id: row.id,
            price: row.price.0,
            purchased_at: row.purchased_at,
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(offers))
    }

    /// Join the queue / reserve this product.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/products/:id/queue", method = "put")]
    async fn queue(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<PurchaseStatus>> {
        queue::queue(self, user, QueueRequest { ticket_kind: id })
            .await
            .map(Json)
    }

    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/queue", method = "get")]
    async fn queue_status(&self, user: User) -> MinilithResult<Json<QueueResponse>> {
        queue::status(self, user).await.map(Json)
    }

    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/queue", method = "delete")]
    async fn drop_queue(&self, user: User) -> MinilithResult<()> {
        queue::drop_transaction_flow(self, user).await
    }

    /// Buy the reserved product.
    ///
    /// With `reader_id`, the payment starts on that `SumUp` reader and the reservation flow is
    /// bypassed completely: no reservation is consumed, and an existing reservation is left to
    /// time out. Clients that pay on a reader must not queue/reserve first.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/products/:id/purchase", method = "post")]
    async fn purchase(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<AppPurchaseRequest>,
    ) -> MinilithResult<Json<BuyTicketResponse>> {
        purchase::begin(
            self,
            user,
            BuyTicketRequest {
                ticket_kind: id,
                provider: body.provider,
                addons: body
                    .addons
                    .into_iter()
                    .map(|addon| BoughtAddon {
                        id: addon.addon_id,
                        selected_options: Some(addon.selected_options),
                        selected_text: addon.selected_text,
                    })
                    .collect(),
                redirect_url: body.redirect_url,
                reader_id: body.reader_id,
            },
        )
        .await
        .map(Json)
    }

    /// Transfer an owned purchase to another user.
    ///
    /// A used purchase cannot be transferred (see `POST /purchases/{id}/use`).
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/purchases/:id/transfer", method = "post")]
    async fn transfer(
        &self,
        auth: User,
        Path(id): Path<Uuid>,
        Json(body): Json<AppTransferRequest>,
    ) -> MinilithResult<()> {
        transfer::transfer(
            self,
            auth,
            transfer::TransferRequest {
                purchased_ticket_id: id,
                to_user: body.to_user,
            },
        )
        .await
    }

    /// Activities the caller can scan purchases for.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/validatable-activities", method = "get")]
    async fn validatable_activities(
        &self,
        auth: User,
    ) -> MinilithResult<Json<Vec<validation::ValidateActivity>>> {
        validation::validatable_activities(self, auth)
            .await
            .map(Json)
    }

    /// Validate (scan) a use.
    ///
    /// `use_id` is a `purchase_uses.id` from `POST /purchases/{id}/use`. Scanning
    /// records a `purchase_use_validations` row; it does not consume a use. Repeat
    /// scans are allowed and returned as `previous_verifications`. A used purchase
    /// can no longer be transferred or resold.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/purchase-uses/validate", method = "post")]
    async fn validate(
        &self,
        auth: User,
        Json(body): Json<ValidateUseRequest>,
    ) -> MinilithResult<Json<ValidateResponse>> {
        validation::validate(self, auth, body.use_id, body.created_at)
            .await
            .map(Json)
    }

    /// Fetch an activity and record that the user viewed it.
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/activities/:id", method = "get")]
    async fn activity(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<crate::activities::Activity>> {
        crate::activities::activity_details(self, user, id)
            .await
            .map(Json)
    }

    /// Notifications that were sent for this activity.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/activities/:id/notifications", method = "get")]
    async fn activity_notifications(
        &self,
        user: User,
        Path(id): Path<Uuid>,
    ) -> MinilithResult<Json<Vec<AppActivityNotification>>> {
        self.test_activity_access(user.get_id(), &id).await?;
        let notifications = sqlx::query!(
            r#"select notification.id,
                notification.title as "title!: DIS",
                notification.content as "content!: DIS",
                notification.send_at
            from push_notifications notification
            inner join activity_push_notifications activity_push
                on activity_push.notification_id = notification.id
            where activity_push.activity_id = $1
            and notification.sent
            order by notification.send_at desc"#,
            id
        )
        .map(|row| AppActivityNotification {
            id: row.id,
            title: row.title.0,
            content: row.content.0,
            send_at: row.send_at,
        })
        .fetch_all(&self.db)
        .await?;
        Ok(Json(notifications))
    }

    /// Tell the organiser that the owner will not attend (shown to admins only).
    ///
    /// # Errors
    ///
    /// AUTH, DB, NOT FOUND.
    #[oai(path = "/purchases/:id/no-show-hint", method = "put")]
    async fn put_no_show_hint(
        &self,
        user: User,
        Path(id): Path<Uuid>,
        Json(body): Json<AppNoShowHint>,
    ) -> MinilithResult<()> {
        let result = sqlx::query!(
            "update purchases set no_show_hint = $3 where id = $1 and owner_id = $2",
            id,
            user.get_id(),
            body.no_show_hint,
        )
        .execute(&self.db)
        .await?;
        if result.rows_affected() == 0 {
            return Err(MinilithEndpointError::bad_frontend_code(
                "you do not own this purchase",
                "",
            ));
        }
        Ok(())
    }

    /// Notifications addressed to the caller, newest first.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/notifications", method = "get")]
    async fn notifications(&self, user: User) -> MinilithResult<Json<Vec<AppNotification>>> {
        let notifications = load_notification_history(&self.db, user.get_id()).await?;
        Ok(Json(
            notifications
                .into_iter()
                .map(|notification| AppNotification {
                    id: notification.id,
                    kind: notification.kind,
                    target_id: notification.target_id,
                    sender: notification.sender.0,
                    title: notification.title.0,
                    content: notification.content.0,
                    send_at: notification.send_at,
                    read: notification.read,
                })
                .collect(),
        ))
    }

    /// Mark these notifications as read.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/notifications/read", method = "put")]
    async fn mark_notifications_read(
        &self,
        user: User,
        Json(body): Json<AppMarkNotificationsRead>,
    ) -> MinilithResult<()> {
        sqlx::query!(
            r#"insert into notification_reads (user_id, notification_id)
            select $1, recipient.notification_id
            from notification_recipients recipient
            where recipient.user_id = $1
            and recipient.notification_id = any($2)
            on conflict do nothing"#,
            user.get_id(),
            &body.ids,
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Mark everything sent so far as read.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/notifications/read-all", method = "put")]
    async fn mark_all_notifications_read(&self, user: User) -> MinilithResult<()> {
        sqlx::query!(
            r#"insert into user_notification_state (user_id, last_full_read)
            values ($1, now())
            on conflict (user_id) do update set last_full_read = excluded.last_full_read"#,
            user.get_id(),
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Record an interaction with a push notification.
    ///
    /// # Errors
    ///
    /// AUTH, DB.
    #[oai(path = "/notification-events", method = "post")]
    async fn notification_event(
        &self,
        user: User,
        Json(body): Json<AppNotificationEvent>,
    ) -> MinilithResult<()> {
        let kind = body.kind;
        let mut txn = self.db.begin().await?;
        sqlx::query!(
            r#"insert into notification_events
                (notification_id, user_id, kind, target_id)
            values ($1, $2, $3::text::notification_event_kind, $4)"#,
            body.notification_id,
            user.get_id(),
            match kind {
                NotificationEventKind::Sent => "sent",
                NotificationEventKind::Opened => "opened",
                NotificationEventKind::Dismissed => "dismissed",
            },
            body.target_id,
        )
        .execute(&mut txn.executor())
        .await?;
        let (opened, dismissed, sent) = match kind {
            NotificationEventKind::Opened => (1, 0, 0),
            NotificationEventKind::Dismissed => (0, 1, 0),
            NotificationEventKind::Sent => (0, 0, 1),
        };
        sqlx::query!(
            r#"insert into notification_stats
                (notification_id, n_opened, n_dismissed, n_sent)
            values ($1, $2, $3, $4)
            on conflict (notification_id) do update set
                n_opened = notification_stats.n_opened + excluded.n_opened,
                n_dismissed = notification_stats.n_dismissed + excluded.n_dismissed,
                n_sent = notification_stats.n_sent + excluded.n_sent"#,
            body.notification_id,
            opened,
            dismissed,
            sent,
        )
        .execute(&mut txn.executor())
        .await?;
        txn.commit().await?;
        Ok(())
    }
}
