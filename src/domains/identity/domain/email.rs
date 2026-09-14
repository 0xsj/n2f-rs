use crate::shared::errors::{Failure, Kind};
/// Canonical login identifier (A01): lowercased ASCII dot-atom local part and DNS
/// domain. Private data: Display and Debug redact; `reveal` is the explicit projection.
#[derive(Clone, PartialEq, Eq)]
pub struct Email {
    value: String,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid email").with_type("identity.email_invalid")
}
fn atext(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&b)
}
fn local_part_ok(local: &[u8]) -> bool {
    !local.is_empty()
        && local.len() <= 64
        && local.first() != Some(&b'.')
        && local.last() != Some(&b'.')
        && !local.windows(2).any(|w| w == b"..")
        && local.iter().all(|&b| b == b'.' || atext(b))
}
fn label_ok(label: &[u8]) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label.first() != Some(&b'-')
        && label.last() != Some(&b'-')
        && label
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
}
impl Email {
    pub fn parse(value: &str) -> Result<Self, Failure> {
        let bytes = value.as_bytes();
        if bytes.is_empty() || bytes.len() > 254 || !bytes.is_ascii() {
            return Err(invalid());
        }
        let at = bytes.iter().position(|&b| b == b'@').ok_or_else(invalid)?;
        let (local, domain) = (&bytes[..at], &bytes[at + 1..]);
        if domain.contains(&b'@') || !local_part_ok(local) {
            return Err(invalid());
        }
        let labels: Vec<&[u8]> = domain.split(|&b| b == b'.').collect();
        if labels.len() < 2 || !labels.iter().all(|l| label_ok(l)) {
            return Err(invalid());
        }
        Ok(Self {
            value: value.to_ascii_lowercase(),
        })
    }
    /// Explicit disclosure of the canonical private text.
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
