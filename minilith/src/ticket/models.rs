#![allow(
    clippy::field_scoped_visibility_modifiers,
    reason = "API DTOs are constructed by sibling ticket modules"
)]

use poem_openapi::{Enum, Object};
use sqlx::types::time::OffsetDateTime;
use uuid::Uuid;

use crate::InternationalizedString as IS;
use crate::activities::PoemLocation;

#[derive(Debug, Clone, Copy, Enum, PartialEq, Eq)]
#[oai(rename_all = "lowercase")]
pub enum PurchaseProvider {
    Swish,
    Stripe,
    Free,
}
#[derive(Debug, Clone, Object)]
pub struct BoughtAddon {
    pub(super) id: Uuid,
    pub(super) selected_text: Option<String>,
    pub(super) selected_options: Option<Vec<i32>>,
}
#[derive(Debug, Clone, Object)]
pub struct BuyTicketRequest {
    pub(super) ticket_kind: Uuid,
    /// Doesn't matter for free tickets.
    pub(super) provider: PurchaseProvider,
    pub(super) addons: Vec<BoughtAddon>,
    /// Required for stripe.
    pub(super) stripe_success_url: Option<String>,
}
#[derive(Debug, Clone, Object)]
pub struct BuyTicketResponse {
    /// Not null when using [`PurchaseProvider::Swish`].
    pub(super) payment_request_token: Option<String>,
    /// Not null when using [`PurchaseProvider::Stripe`].
    /// Open this in a new browser context.
    /// Close that context when [`BuyTicketRequest::stripe_success_url`] is reached.
    pub(super) stripe_url: Option<String>,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct QueueRequest {
    pub(super) ticket_kind: Uuid,
}
#[derive(Debug, Clone, Copy, Object)]
pub struct QueueResponse {
    pub(super) ticket_kind: Uuid,
    /// A placement of 0 indicates you can buy the ticket.
    /// `None` indicates the tickets have not yet been released.
    pub(super) placement: Option<i32>,
    /// When the ticket will be made unavailable for purchase, i.e. the reservation ran out.
    /// Will be not null when placement is 0.
    pub(super) timeout: Option<OffsetDateTime>,
    /// When transactions at latest should be conducted.
    /// Will be not null when placement is 0.
    pub(super) start_transaction_before: Option<OffsetDateTime>,
}
#[derive(Debug, Clone, Copy, Enum)]
pub enum PurchaseStatus {
    /// Standing in release queue (tickets have not been released yet).
    /// Request the queue endpoint to get queue status.
    ReleaseQueued,
    /// Standing in reservation queue (tickets have been released).
    /// Request the queue endpoint to get queue status.
    ReservationQueued,
    /// Ready to be transacted.
    Reserved,
    /// Transaction is happening. Making another transaction request will override the current
    /// transaction.
    Buying,
    /// User owns the ticket now.
    Purchased,
}
#[derive(Object, Debug)]
pub struct Addon {
    pub id: Uuid,
    pub name: IS,
    pub multiple_alternatives: bool,
    pub has_text_field: bool,
    pub required: bool,
}
#[derive(Object, Debug)]
pub struct PurchasedAddon {
    #[oai(flatten)]
    pub inner: Addon,
    pub options: Vec<AddonOption>,
    pub selected_options: Vec<i32>,
    pub selected_text: String,
}
#[derive(Object, Clone, Debug)]
pub struct AddonOption {
    pub id: Uuid,
    pub idx: i32,
    pub name: IS,
    pub price: i64,
    // for admins mostly
    pub bookkeeping_prices: Vec<i64>,
    pub bookkeeping_price_categories: Vec<String>,
}
#[derive(Object, Debug)]
pub struct AvailableAddon {
    #[oai(flatten)]
    pub inner: Addon,
    pub options: Vec<AddonOption>,
}

#[allow(clippy::module_name_repetitions, reason = "Base is a shit name")]
#[derive(Object, Debug)]
pub struct TicketBase {
    #[allow(clippy::struct_field_names, reason = "reasonable name")]
    pub ticket_kind_id: Uuid,
    #[allow(clippy::struct_field_names, reason = "reasonable name")]
    pub ticket_kind_name: IS,
    pub activity_id: Uuid,
}
#[derive(Object, Debug)]
pub(super) struct PurchasedTicket {
    #[oai(flatten)]
    pub(super) inner: TicketBase,
    pub(super) id: Uuid,
    pub(super) activity_location: PoemLocation,
    pub(super) activity_title: IS,
    pub(super) creator_id: Uuid,
    pub(super) creator_path: String,
    pub(super) creator_name: IS,
    pub(super) time_start: OffsetDateTime,
    pub(super) time_end: OffsetDateTime,
    pub(super) purchased_addons: Vec<PurchasedAddon>,
    /// False if we have transferred it.
    pub(super) owned_by_me: bool,
}
#[derive(Object, Debug)]
pub struct Kind {
    #[oai(flatten)]
    pub inner: TicketBase,
    pub price: i64,
    pub purchasing_available_start: OffsetDateTime,
    pub purchasing_available_stop: OffsetDateTime,
    pub max_tickets: i32,
    pub min_tickets: i32,
    pub reserved_or_purchased_tickets: i32,
    pub allow_transfer_ticket_start: OffsetDateTime,
    pub allow_transfer_ticket_stop: OffsetDateTime,
    /// A recipient must belong to one of these groups or a descendant.
    pub transfer_group_ids: Vec<Uuid>,
    pub has_been_purchased: bool,
    pub has_been_released: bool,
    pub allowed_group_ids: Vec<Uuid>,
    pub available_addons: Vec<AvailableAddon>,
}

impl Kind {
    pub(crate) fn activity_id(&self) -> Uuid {
        self.inner.activity_id
    }

    pub(crate) fn reserved_or_purchased_tickets(&self) -> i32 {
        self.reserved_or_purchased_tickets
    }

    pub(crate) fn has_been_purchased(&self) -> bool {
        self.has_been_purchased
    }

    pub(crate) fn immutable_fields_match(
        &self,
        activity_id: Uuid,
        price: i64,
        allowed_group_ids: &[Uuid],
        addons: &[AvailableAddon],
    ) -> bool {
        self.inner.activity_id == activity_id
            && self.price == price
            && self.allowed_group_ids == allowed_group_ids
            && self.available_addons.len() == addons.len()
            && self.available_addons.iter().zip(addons).all(|(old, new)| {
                old.inner.id == new.inner.id
                    && old.inner.name == new.inner.name
                    && old.inner.multiple_alternatives == new.inner.multiple_alternatives
                    && old.inner.has_text_field == new.inner.has_text_field
                    && old.inner.required == new.inner.required
                    && old.options.len() == new.options.len()
                    && old
                        .options
                        .iter()
                        .zip(&new.options)
                        .all(|(old_option, new_option)| {
                            old_option.id == new_option.id
                                && old_option.name == new_option.name
                                && old_option.price == new_option.price
                        })
            })
    }
}
