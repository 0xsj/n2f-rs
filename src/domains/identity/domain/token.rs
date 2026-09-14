use crate::shared::errors::{Failure, Kind};
/// Closed token purposes (A05). The purpose is part of the stored digest input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenPurpose {
    Session,
    EmailVerification,
    PasswordReset,
    WebSocketUpgrade,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid token value").with_type("identity.token_invalid")
}
impl TokenPurpose {
    pub fn parse(value: &str) -> Result<Self, Failure> {
        match value {
            "session" => Ok(Self::Session),
            "email_verification" => Ok(Self::EmailVerification),
            "password_reset" => Ok(Self::PasswordReset),
            "websocket_upgrade" => Ok(Self::WebSocketUpgrade),
            _ => Err(invalid()),
        }
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
/// A validated 32-byte digest with its purpose. Parsing proves shape only: it does
/// not hash, generate tokens or prove the source token came from a CSPRNG.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TokenDigest {
    purpose: TokenPurpose,
    data: [u8; 32],
}
impl TokenDigest {
    pub fn parse(purpose: TokenPurpose, data: &[u8]) -> Result<Self, Failure> {
        let data: [u8; 32] = data.try_into().map_err(|_| invalid())?;
        Ok(Self { purpose, data })
    }
    pub fn purpose(&self) -> TokenPurpose {
        self.purpose
    }
    /// Owned copy; fixed arrays copy by value.
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
