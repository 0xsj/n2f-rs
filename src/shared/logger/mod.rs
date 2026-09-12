//! Process logger adapters, explicit projections and bounded delivery.
#![doc = include_str!("CONTRACT.md")]
mod config;
mod delivery;
mod formatter;
mod projection;
mod runtime;
mod value;
pub use config::{Config, Level, Resource, color_enabled, parse_level};
pub use delivery::Stats;
pub use runtime::{Logger, Runtime};
pub use value::{Fields, Value};
