//! Shared foundations with a concrete consumer and named responsibility.
//!
//! A consumer may be the process or composition root before any domain exists.
//! Keep business-specific behavior with its domain owner.
//! Shared code must not depend on business modules or the composition root.
//! No logger, provenance model, or common repository abstraction is defined yet.
//!
//! The first leaf foundation lives in [`errors`], with a documented public
//! contract and tests for classification, ownership and diagnostic preservation.

pub mod errors;

pub mod clock;
pub mod id;
