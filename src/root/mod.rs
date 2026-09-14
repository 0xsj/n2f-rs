//! Foundations configuration, composition and process lifecycle.
//! Shared and product modules must not import this boundary.
mod audit;
mod auth;
pub mod config;
mod demo;
pub mod events;
pub mod http;
mod logging;
mod proxy;
pub use demo::run;
