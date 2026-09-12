use std::fmt;

/// Owned text with redacted display and an explicit disclosure boundary.
/// Private storage cannot be initialized or read by consumers.
/// ```compile_fail
/// use n2f_rs::shared::secret::SecretString;
/// let value = SecretString::new("private".into());
/// let raw = value.value;
/// ```
/// ```compile_fail
/// use n2f_rs::shared::secret::SecretString;
/// let raw: String = SecretString::new("private".into());
/// ```
pub struct SecretString {
    value: String,
}
impl SecretString {
    pub fn new(value: String) -> Self {
        Self { value }
    }
    pub fn reveal(&self) -> &str {
        &self.value
    }
}
impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
