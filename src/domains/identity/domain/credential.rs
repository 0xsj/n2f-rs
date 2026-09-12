use super::{auth_spec_stub::pending, email::Email};
use crate::shared::{errors::Failure, id::Id, secret::SecretString};
pub struct CredentialSnapshot {
    pub principal_id: Id,
    pub email: Email,
    pub password_hash: SecretString,
    pub verified_at_ms: Option<i64>,
    pub password_version: u32,
    pub created_at_ms: i64,
    pub changed_at_ms: i64,
}
pub struct PasswordCredential {
    state: CredentialSnapshot,
}
impl PasswordCredential {
    pub fn restore(_state: CredentialSnapshot) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn snapshot(&self) -> CredentialSnapshot {
        CredentialSnapshot {
            principal_id: self.state.principal_id,
            email: self.state.email.clone(),
            password_hash: SecretString::new(self.state.password_hash.reveal().to_owned()),
            verified_at_ms: self.state.verified_at_ms,
            password_version: self.state.password_version,
            created_at_ms: self.state.created_at_ms,
            changed_at_ms: self.state.changed_at_ms,
        }
    }
}
