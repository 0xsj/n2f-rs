//! Wall timestamps and monotonic elapsed readings, with explicit test control.
//!
//! Supply timestamps to domain functions as values. Applications obtain them
//! through the narrow capability they consume. SystemClock uses SystemTime and
//! Instant; FakeClock separates wall correction from elapsed progress.
//!
//! The local acceptance contract includes ownership, failure and precision:
#![doc = include_str!("CONTRACT.md")]

mod fake;
mod system;
pub use fake::{AdvanceError, FakeClock};
pub use system::SystemClock;
