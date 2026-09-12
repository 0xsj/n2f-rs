use super::{
    auth_spec_stub::pending,
    token::{TokenDigest, TokenPurpose},
};
use crate::shared::{errors::Failure, id::Id};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChallengeSnapshot {
    pub id: Id,
    pub principal_id: Id,
    pub token_digest: TokenDigest,
    pub password_version: u32,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub consumed_at_ms: Option<i64>,
    pub invalidated_at_ms: Option<i64>,
}
pub struct Challenge {
    state: ChallengeSnapshot,
}
impl Challenge {
    pub fn issue(
        _id: Id,
        _principal_id: Id,
        _digest: TokenDigest,
        _version: u32,
        _issued: i64,
        _expires: i64,
    ) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn restore(_state: ChallengeSnapshot) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn snapshot(&self) -> ChallengeSnapshot {
        self.state.clone()
    }
    pub fn consume(
        &self,
        _purpose: TokenPurpose,
        _version: u32,
        _now: i64,
    ) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn invalidate(&self, _now: i64) -> Result<Self, Failure> {
        Err(pending())
    }
}
