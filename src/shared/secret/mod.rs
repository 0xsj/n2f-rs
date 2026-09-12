//! An implementation of an owned secret string with redacted presentation.
//!
//! This leaf owns private string storage, explicit reveal and manual
//! Display/Debug implementations. Empty strings are valid; env/config owns
//! requiredness. No project module, serializer or provider dependency is needed.
//!
//! Runtime APIs and executable tests are implemented. Logger projection
//! is verified in the logger adapter tests. Redaction does not protect process memory.
#![doc = include_str!("CONTRACT.md")]

mod value;
pub use value::SecretString;
