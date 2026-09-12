use super::{auth_spec_stub::pending, token::TokenDigest};
use crate::shared::{errors::Failure, id::Id};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub id: Id,
    pub principal_id: Id,
    pub token_digest: TokenDigest,
    pub auth_epoch: u32,
    pub issued_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub absolute_expires_at_ms: i64,
    pub idle_expires_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}
pub struct Session {
    state: SessionSnapshot,
}
impl Session {
    pub fn issue(
        _id: Id,
        _principal_id: Id,
        _digest: TokenDigest,
        _epoch: u32,
        _issued: i64,
        _absolute: i64,
        _idle: i64,
    ) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn restore(_state: SessionSnapshot) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn snapshot(&self) -> SessionSnapshot {
        self.state.clone()
    }
    pub fn check(&self, _now: i64) -> Result<(), Failure> {
        Err(pending())
    }
    pub fn touch(&self, _now: i64, _idle_ttl: i64) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn revoke(&self, _now: i64) -> Result<Self, Failure> {
        Err(pending())
    }
}
