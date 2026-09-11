use std::collections::BTreeMap;
use std::error::Error as StdError;

use super::Kind;

/// A borrowed classification frame. public_info applies the disclosure policy.
#[derive(Debug, Clone, Copy)]
pub struct Classification<'a> {
    pub kind: Kind,
    pub error_type: Option<&'a str>,
    pub message: &'a str,
    pub fields: Option<&'a BTreeMap<String, String>>,
}
/// Explicit classification supplied by the owner; no registry of domain types.
pub trait Classified: StdError {
    fn classification(&self) -> Option<Classification<'_>> {
        None
    }
}
