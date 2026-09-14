//! Shared foundations with a concrete consumer and named responsibility.
//!
//! A consumer may be the process or composition root before any domain exists.
//! Keep business-specific behavior with its domain owner.
//! Shared code must not depend on business modules or the composition root.
//! Provenance and process logging implement their local contracts.
//! There is no common repository abstraction.
//!
//! The first leaf foundation lives in [`errors`], with a documented public
//! contract and tests for classification, ownership and diagnostic preservation.

pub mod errors;

pub mod clock;
pub mod entropy;
pub mod env;
pub mod id;
pub mod keyed;
pub mod logger;
pub mod provenance;
pub mod secret;

pub mod events;
pub mod health;
pub mod http;
pub mod httpclient;
pub mod pagination;
pub mod postgres;
pub mod socket;
pub mod telemetry;
pub mod validation;
