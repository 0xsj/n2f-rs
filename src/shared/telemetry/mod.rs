//! Trace identity, outcomes and owned telemetry delivery.
//!
//! Value leaves for tracing identity and observed outcomes. SDK providers remain
//! concrete adapters in the otel submodule, separate from these value leaves.
#![doc = include_str!("CONTRACT.md")]

mod context;
pub mod otel;
mod outcome;
pub use context::{TraceRef, TraceSnapshot};
pub use outcome::Outcome;
