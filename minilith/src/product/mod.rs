//! Product operations (tickets are one kind of product) and their HTTP API.
//!
//! [`legacy`] contains the deprecated `/tickets` endpoints kept for older clients.
//! It only maps the old DTOs onto the same operations as the product API, so it
//! can be deleted in one go once clients have moved.
//!
//! # Authorization invariants
//!
//! [`api`] is only a transport boundary: it extracts authenticated callers and
//! request data, delegates to an operation module, and wraps the response. Each
//! operation must enforce all authorization and ownership requirements itself;
//! callers must never compensate for a missing operation-level check.
//!
//! Reusable authorization-policy queries belong in [`access`]. Checks that
//! depend on mutable purchase state belong beside the protected operation and
//! must run in the same database transaction. Functions whose names end in
//! `_unchecked` may only be called after the caller has established and
//! documented the required authorization.

use std::ops::Deref;

use crate::{ContextWrapper, MinilithEndpointError, MinilithResult};

mod access;
mod allocation;
pub mod app;
pub(crate) mod catalog;
mod flow;
mod legacy;
mod models;
mod notifications;
mod purchase;
mod queue;
mod release;
mod sales;
#[cfg(test)]
mod tests;
mod transfer;
mod validation;

#[allow(
    clippy::module_name_repetitions,
    reason = "preserves the public ticket API after splitting the module"
)]
pub use models::{
    Addon, AddonOption, AvailableAddon, Kind, PurchaseProvider, PurchaseStatus, PurchasedAddon,
    TicketBase,
};

#[derive(Debug, Clone)]
pub struct Router {
    pub context: ContextWrapper,
}

impl Deref for Router {
    type Target = ContextWrapper;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

pub(crate) use catalog::{load_addons, load_purchasable_addons};
pub(crate) use purchase::{callback, refund};
pub(crate) use release::check_all_tickets;

/// Cheapest allowed paid price, in ören. Free (0) is always allowed.
pub(crate) const MIN_PRICE: i64 = 1_000;

/// Rejects prices below [`MIN_PRICE`] unless the item is free (`0`), so the
/// payment fee can never exceed what the buyer pays.
pub(crate) fn ensure_price_is_allowed(price: i64) -> MinilithResult<()> {
    if price != 0 && price < MIN_PRICE {
        return Err(MinilithEndpointError::bad_frontend_code(
            "prices must be free (0) or at least 10 SEK",
            price,
        ));
    }
    Ok(())
}

fn ensure_affected_rows(
    affected: u64,
    expected: usize,
    operation: &'static str,
) -> MinilithResult<()> {
    if usize::try_from(affected).ok() == Some(expected) {
        Ok(())
    } else {
        Err(MinilithEndpointError::internal_error(
            operation,
            format!("expected {expected} affected rows, got {affected}"),
        ))
    }
}
