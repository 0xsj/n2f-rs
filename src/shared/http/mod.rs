//! HTTP ingress, safe failure responses and request completion.
//!
//! Pure projection/policy leaves and concrete native adapters are implemented.
//! See API.md and the root run guide for ownership and actual verification.
#![doc = include_str!("CONTRACT.md")]
pub mod axum;
mod lifecycle;
pub mod otel;
mod policy;
pub use lifecycle::{Active, Completion};
mod problem;
pub use policy::{
    Classification, CompletionFacts, Outcome, Termination, classify_completion, normalize_method,
};
pub use problem::{Problem, problem_of};
