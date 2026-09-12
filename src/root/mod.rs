//! Foundations configuration, composition and process lifecycle.
//! Shared and product modules must not import this boundary.
pub mod config;
mod demo;
pub mod events;
pub mod http;
mod logging;
pub use demo::run;
