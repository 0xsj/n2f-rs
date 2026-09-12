use super::auth_spec_stub::pending;
use crate::shared::errors::Failure;
#[derive(Clone)]
pub struct Email {
    value: String,
}
impl Email {
    pub fn parse(_value: &str) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn reveal(&self) -> &str {
        &self.value
    }
}
impl std::fmt::Display for Email {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl std::fmt::Debug for Email {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
