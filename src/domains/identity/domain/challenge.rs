use super::{
    time_in_range,
    token::{TokenDigest, TokenPurpose},
    version_in_range,
};
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
};
/// Single-use challenge facts (A09) for email verification and password reset.
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
#[derive(Clone, Debug)]
pub struct Challenge {
    state: ChallengeSnapshot,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid challenge").with_type("identity.challenge_invalid")
}
fn rejected() -> Failure {
    Failure::new(Kind::Unauthenticated, "challenge rejected")
        .with_type("identity.challenge_rejected")
}
impl Challenge {
    pub fn issue(
        id: Id,
        principal_id: Id,
        digest: TokenDigest,
        version: u32,
        issued: i64,
        expires: i64,
    ) -> Result<Self, Failure> {
        Self::restore(ChallengeSnapshot {
            id,
            principal_id,
            token_digest: digest,
            password_version: version,
            issued_at_ms: issued,
            expires_at_ms: expires,
            consumed_at_ms: None,
            invalidated_at_ms: None,
        })
    }
    /// Consumption must fall in [issued, expires); invalidation may follow expiry.
    pub fn restore(state: ChallengeSnapshot) -> Result<Self, Failure> {
        let s = &state;
        if !matches!(
            s.token_digest.purpose(),
            TokenPurpose::EmailVerification | TokenPurpose::PasswordReset
        ) || !version_in_range(s.password_version)
            || !time_in_range(s.issued_at_ms)
            || !time_in_range(s.expires_at_ms)
            || s.expires_at_ms <= s.issued_at_ms
            || s.consumed_at_ms.is_some_and(|at| {
                !time_in_range(at) || at < s.issued_at_ms || at >= s.expires_at_ms
            })
            || s.invalidated_at_ms
                .is_some_and(|at| !time_in_range(at) || at < s.issued_at_ms)
        {
            return Err(invalid());
        }
        Ok(Self { state })
    }
    pub fn snapshot(&self) -> ChallengeSnapshot {
        self.state.clone()
    }
    /// Returns a consumed value; only storage can guarantee one winner across callers.
    pub fn consume(&self, purpose: TokenPurpose, version: u32, now: i64) -> Result<Self, Failure> {
        if !time_in_range(now) {
            return Err(invalid());
        }
        let s = &self.state;
        if purpose != s.token_digest.purpose()
            || version != s.password_version
            || s.consumed_at_ms.is_some()
            || s.invalidated_at_ms.is_some()
            || now < s.issued_at_ms
            || now >= s.expires_at_ms
        {
            return Err(rejected());
        }
        let mut state = s.clone();
        state.consumed_at_ms = Some(now);
        Ok(Self { state })
    }
    /// Idempotent; preserves any consumed time and the first invalidation time.
    pub fn invalidate(&self, now: i64) -> Result<Self, Failure> {
        if !time_in_range(now) || now < self.state.issued_at_ms {
            return Err(invalid());
        }
        let mut state = self.state.clone();
        if state.invalidated_at_ms.is_none() {
            state.invalidated_at_ms = Some(now);
        }
        Ok(Self { state })
    }
}
