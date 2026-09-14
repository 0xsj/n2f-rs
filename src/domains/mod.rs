//! Business modules, organized by the domain that owns their behavior.
//!
//! Add a named module for a real workflow. Keep its business rules, application
//! operations, infrastructure adapters, and transports within that module.
//! Identity currently owns pure principal values and versioned lifecycle rules.
//! Application operations and adapters follow the repository's DOMAINS.md plan.
pub mod audit;
pub mod identity;
pub mod org;
