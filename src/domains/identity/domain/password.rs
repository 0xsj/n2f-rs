use crate::shared::{
    errors::{Failure, Kind},
    secret::SecretString,
};
use unicode_normalization::UnicodeNormalization;
/// Raw input ceiling applied before NFC normalization, for enrollment and login.
const INPUT_MAX_BYTES: usize = 4096;
/// Enrollment policy (A02) after NFC: 15..=128 scalar values and at most 512 bytes.
const ENROLL_MIN_SCALARS: usize = 15;
const ENROLL_MAX_SCALARS: usize = 128;
const ENROLL_MAX_BYTES: usize = 512;
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid password").with_type("identity.password_invalid")
}
/// Rust `String` is always valid Unicode; the malformed-input case cannot reach here.
fn normalize(value: SecretString) -> Result<String, Failure> {
    let raw = value.reveal();
    if raw.is_empty() || raw.len() > INPUT_MAX_BYTES {
        return Err(invalid());
    }
    let normalized: String = raw.nfc().collect();
    if normalized.is_empty() || normalized.len() > INPUT_MAX_BYTES {
        return Err(invalid());
    }
    Ok(normalized)
}
/// A password accepted for enrollment: creation, change or reset. Blocklist
/// admission is a separate application port.
pub struct NewPassword {
    value: SecretString,
}
/// A password presented at login: normalized and bounded, but not held to the
/// current enrollment policy.
pub struct PasswordInput {
    value: SecretString,
}
impl NewPassword {
    pub fn parse(value: SecretString) -> Result<Self, Failure> {
        let normalized = normalize(value)?;
        let scalars = normalized.chars().count();
        if !(ENROLL_MIN_SCALARS..=ENROLL_MAX_SCALARS).contains(&scalars)
            || normalized.len() > ENROLL_MAX_BYTES
        {
            return Err(invalid());
        }
        Ok(Self {
            value: SecretString::new(normalized),
        })
    }
    pub fn secret(&self) -> &SecretString {
        &self.value
    }
}
impl PasswordInput {
    pub fn parse(value: SecretString) -> Result<Self, Failure> {
        Ok(Self {
            value: SecretString::new(normalize(value)?),
        })
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
