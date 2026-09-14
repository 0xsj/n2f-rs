use super::{
    time_in_range,
    token::{TokenDigest, TokenPurpose},
};
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
};

/// Short-lived, single-use session-bound WebSocket admission facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeTicketSnapshot {
    pub id: Id,
    pub session_id: Id,
    pub token_digest: TokenDigest,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub consumed_at_ms: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct UpgradeTicket {
    state: UpgradeTicketSnapshot,
}
const MAX_TTL_MS: i64 = 30_000;
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid websocket upgrade ticket")
        .with_type("identity.upgrade_ticket_invalid")
}
fn rejected() -> Failure {
    Failure::new(Kind::Unauthenticated, "websocket upgrade ticket rejected")
        .with_type("identity.upgrade_ticket_rejected")
}
impl UpgradeTicket {
    pub fn issue(
        id: Id,
        session_id: Id,
        token_digest: TokenDigest,
        issued_at_ms: i64,
        expires_at_ms: i64,
    ) -> Result<Self, Failure> {
        Self::restore(UpgradeTicketSnapshot {
            id,
            session_id,
            token_digest,
            issued_at_ms,
            expires_at_ms,
            consumed_at_ms: None,
        })
    }
    pub fn restore(state: UpgradeTicketSnapshot) -> Result<Self, Failure> {
        if state.token_digest.purpose() != TokenPurpose::WebSocketUpgrade
            || !time_in_range(state.issued_at_ms)
            || !time_in_range(state.expires_at_ms)
            || state.expires_at_ms <= state.issued_at_ms
            || state.expires_at_ms - state.issued_at_ms > MAX_TTL_MS
            || state.consumed_at_ms.is_some_and(|at| {
                !time_in_range(at) || at < state.issued_at_ms || at >= state.expires_at_ms
            })
        {
            return Err(invalid());
        }
        Ok(Self { state })
    }
    pub fn snapshot(&self) -> UpgradeTicketSnapshot {
        self.state.clone()
    }
    pub fn consume(&self, now: i64) -> Result<Self, Failure> {
        if !time_in_range(now) {
            return Err(invalid());
        }
        if self.state.consumed_at_ms.is_some()
            || now < self.state.issued_at_ms
            || now >= self.state.expires_at_ms
        {
            return Err(rejected());
        }
        let mut state = self.state.clone();
        state.consumed_at_ms = Some(now);
        Ok(Self { state })
    }
}
