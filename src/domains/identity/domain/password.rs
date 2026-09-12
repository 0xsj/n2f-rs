use super::auth_spec_stub::pending;
use crate::shared::{errors::Failure, secret::SecretString};
pub struct NewPassword {
    value: SecretString,
}
pub struct PasswordInput {
    value: SecretString,
}
impl NewPassword {
    pub fn parse(_value: SecretString) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn secret(&self) -> &SecretString {
        &self.value
    }
}
impl PasswordInput {
    pub fn parse(_value: SecretString) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn secret(&self) -> &SecretString {
        &self.value
    }
}
impl std::fmt::Display for NewPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl std::fmt::Debug for NewPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl std::fmt::Display for PasswordInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl std::fmt::Debug for PasswordInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
