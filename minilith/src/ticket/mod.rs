//! Ticket operations and their HTTP API.
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
mod api;
mod catalog;
mod flow;
mod models;
mod purchase;
mod queue;
mod release;
#[cfg(test)]
mod tests;
mod transfer;
mod validation;

#[allow(
    clippy::module_name_repetitions,
    reason = "preserves the public ticket API after splitting the module"
)]
pub use models::{Addon, AddonOption, AvailableAddon, Kind, TicketBase};

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

pub(crate) use catalog::load_ticket_kind_unchecked;
pub(crate) use release::check_all_tickets;

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
