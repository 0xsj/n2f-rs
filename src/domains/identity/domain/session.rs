use super::{
    time_in_range,
    token::{TokenDigest, TokenPurpose},
    version_in_range,
};
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
};
/// Session facts (A06). IDs are public; the token digest is redacted by its type.
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
#[derive(Clone, Debug)]
pub struct Session {
    state: SessionSnapshot,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid session").with_type("identity.session_invalid")
}
fn rejected() -> Failure {
    Failure::new(Kind::Unauthenticated, "session rejected").with_type("identity.session_rejected")
}
impl Session {
    pub fn issue(
        id: Id,
        principal_id: Id,
        digest: TokenDigest,
        epoch: u32,
        issued: i64,
        absolute: i64,
        idle: i64,
    ) -> Result<Self, Failure> {
        Self::restore(SessionSnapshot {
            id,
            principal_id,
            token_digest: digest,
            auth_epoch: epoch,
            issued_at_ms: issued,
            last_seen_at_ms: issued,
            absolute_expires_at_ms: absolute,
            idle_expires_at_ms: idle,
            revoked_at_ms: None,
        })
    }
    /// Validates issued <= lastSeen < absolute, lastSeen <= idle <= absolute, both
    /// deadlines strictly after issued, and revocation no earlier than issue.
    pub fn restore(state: SessionSnapshot) -> Result<Self, Failure> {
        let s = &state;
        if s.token_digest.purpose() != TokenPurpose::Session
            || !version_in_range(s.auth_epoch)
            || !time_in_range(s.issued_at_ms)
            || !time_in_range(s.last_seen_at_ms)
            || !time_in_range(s.absolute_expires_at_ms)
            || !time_in_range(s.idle_expires_at_ms)
            || s.last_seen_at_ms < s.issued_at_ms
            || s.absolute_expires_at_ms <= s.issued_at_ms
            || s.idle_expires_at_ms <= s.issued_at_ms
            || s.idle_expires_at_ms > s.absolute_expires_at_ms
            || s.last_seen_at_ms > s.idle_expires_at_ms
            || s.last_seen_at_ms >= s.absolute_expires_at_ms
            || s.revoked_at_ms
                .is_some_and(|at| !time_in_range(at) || at < s.issued_at_ms)
        {
            return Err(invalid());
        }
        Ok(Self { state })
    }
    pub fn snapshot(&self) -> SessionSnapshot {
        self.state.clone()
    }
    /// Local session validity only (A07): not revoked, not backward, and before
    /// both deadlines. Equality at a deadline is expired. Principal, credential and
    /// epoch eligibility are application checks.
    pub fn check(&self, now: i64) -> Result<(), Failure> {
        if !time_in_range(now) {
            return Err(invalid());
        }
        let s = &self.state;
        if s.revoked_at_ms.is_some()
            || now < s.last_seen_at_ms
            || now >= s.idle_expires_at_ms
            || now >= s.absolute_expires_at_ms
        {
            return Err(rejected());
        }
        Ok(())
    }
    /// Advances lastSeen and idle expiry to min(now + ttl, absolute); the absolute
    /// deadline never slides.
    pub fn touch(&self, now: i64, idle_ttl: i64) -> Result<Self, Failure> {
        self.check(now)?;
        if idle_ttl < 1 {
            return Err(invalid());
        }
        let idle = now.checked_add(idle_ttl).ok_or_else(invalid)?;
        let mut state = self.state.clone();
        state.last_seen_at_ms = now;
        state.idle_expires_at_ms = idle.min(state.absolute_expires_at_ms);
        Ok(Self { state })
    }
    /// Idempotent; keeps the first revocation time. Expired sessions may still be
    /// revoked so logout latches a terminal fact.
    pub fn revoke(&self, now: i64) -> Result<Self, Failure> {
        if !time_in_range(now) || now < self.state.last_seen_at_ms {
            return Err(invalid());
        }
        let mut state = self.state.clone();
        if state.revoked_at_ms.is_none() {
            state.revoked_at_ms = Some(now);
        }
        Ok(Self { state })
    }
}
