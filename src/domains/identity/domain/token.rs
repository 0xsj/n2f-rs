use super::auth_spec_stub::pending;
use crate::shared::errors::Failure;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenPurpose {
    Session,
    EmailVerification,
    PasswordReset,
    WebSocketUpgrade,
}
impl TokenPurpose {
    pub fn parse(_value: &str) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::EmailVerification => "email_verification",
            Self::PasswordReset => "password_reset",
            Self::WebSocketUpgrade => "websocket_upgrade",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TokenDigest {
    purpose: TokenPurpose,
    data: [u8; 32],
}
impl TokenDigest {
    pub fn parse(_purpose: TokenPurpose, _data: &[u8]) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn purpose(&self) -> TokenPurpose {
        self.purpose
    }
    pub fn bytes(&self) -> [u8; 32] {
        self.data
    }
}
impl std::fmt::Display for TokenDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl std::fmt::Debug for TokenDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
