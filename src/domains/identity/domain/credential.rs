use super::{email::Email, time_in_range, version_in_range};
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
    secret::SecretString,
};
/// Stored hash text ceiling. PHC format validation belongs to the password adapter.
const HASH_MAX_BYTES: usize = 512;
/// Owned credential facts (A04). No Debug: the hash is sensitive.
pub struct CredentialSnapshot {
    pub principal_id: Id,
    pub email: Email,
    pub password_hash: SecretString,
    pub verified_at_ms: Option<i64>,
    pub password_version: u32,
    pub created_at_ms: i64,
    pub changed_at_ms: i64,
}
impl CredentialSnapshot {
    fn copied(&self) -> Self {
        Self {
            principal_id: self.principal_id,
            email: self.email.clone(),
            password_hash: SecretString::new(self.password_hash.reveal().to_owned()),
            verified_at_ms: self.verified_at_ms,
            password_version: self.password_version,
            created_at_ms: self.created_at_ms,
            changed_at_ms: self.changed_at_ms,
        }
    }
}
pub struct PasswordCredential {
    state: CredentialSnapshot,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid credential").with_type("identity.credential_invalid")
}
fn corrupt() -> Failure {
    Failure::new(Kind::Internal, "corrupt credential record")
        .with_type("identity.credential_corrupt")
}
impl PasswordCredential {
    /// Validates without resetting history; a present zero timestamp stays present.
    pub fn restore(state: CredentialSnapshot) -> Result<Self, Failure> {
        if !version_in_range(state.password_version)
            || !time_in_range(state.created_at_ms)
            || !time_in_range(state.changed_at_ms)
            || state.changed_at_ms < state.created_at_ms
            || state
                .verified_at_ms
                .is_some_and(|at| !time_in_range(at) || at < state.created_at_ms)
        {
            return Err(invalid());
        }
        let hash = state.password_hash.reveal();
        if hash.is_empty() || hash.len() > HASH_MAX_BYTES {
            return Err(corrupt());
        }
        Ok(Self { state })
    }
    pub fn snapshot(&self) -> CredentialSnapshot {
        self.state.copied()
    }
}
