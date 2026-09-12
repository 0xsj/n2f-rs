use super::actor::invalid;
use crate::shared::errors::Failure;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation(String);
impl Operation {
    pub fn new(s: String) -> Result<Self, Failure> {
        if s.is_empty()
            || s.len() > 128
            || !s.as_bytes()[0].is_ascii_lowercase()
            || !s
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.:-".contains(&b))
        {
            return Err(invalid("invalid_operation"));
        }
        Ok(Self(s))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
