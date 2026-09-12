//! Captured environment parsing and redacted configuration manifests.
//! Root owns settings and lifecycle; this module owns V01-V06.
#![doc = include_str!("CONTRACT.md")]
mod lookup;
mod os;
mod parse;
mod reader;
pub use lookup::map;
pub use os::os;
pub use reader::{Reader, Var};
