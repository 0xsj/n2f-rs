//! UUID values and controllable generation, independent of any framework.
//!
//! Id is a value; V7 owns effects and ordering state. Consumers supply only a
//! wall-time closure. Generation takes an exclusive mutable borrow; callers own
//! synchronization if sharing. Sequence supplies finite deterministic fixtures.
#![doc = include_str!("CONTRACT.md")]

mod sequence;
mod v7;
mod value;
pub use sequence::Sequence;
pub use v7::{Entropy, EntropyError, OsEntropy, V7};
pub use value::Id;
