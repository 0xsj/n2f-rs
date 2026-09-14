//! Identity principal and authentication leaves. See CONTRACT.md and AUTH_CONTRACT.md.
pub mod auth_state;
pub mod challenge;
pub mod credential;
pub mod email;
pub mod password;
pub mod session;
pub mod token;
pub mod upgrade_ticket;
use crate::shared::{
    errors::{Failure, Kind as ErrorKind},
    id::Id,
    validation,
};
/// Common bounds shared by every build: Unix milliseconds and a signed-32-bit
/// version/epoch ceiling, so host numeric ranges do not change domain behavior.
pub const MAX_TIME_MS: i64 = 253402300799999;
pub const MAX_VERSION: u32 = 2147483647;
pub(super) fn time_in_range(at: i64) -> bool {
    (0..=MAX_TIME_MS).contains(&at)
}
pub(super) fn version_in_range(version: u32) -> bool {
    (1..=MAX_VERSION).contains(&version)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Human,
    Service,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Suspended,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub id: Id,
    pub kind: Kind,
    pub display_name: String,
    pub status: Status,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub version: u32,
}
#[derive(Clone, Debug)]
pub struct Principal {
    state: Snapshot,
}
fn invalid() -> Failure {
    Failure::new(ErrorKind::Invalid, "invalid principal").with_type("identity.principal_invalid")
}
impl Principal {
    pub fn register(id: Id, kind: Kind, display_name: String, at: i64) -> Result<Self, Failure> {
        Self::restore(Snapshot {
            id,
            kind,
            display_name,
            status: Status::Active,
            created_at_ms: at,
            updated_at_ms: at,
            version: 1,
        })
    }
    pub fn restore(state: Snapshot) -> Result<Self, Failure> {
        if state.created_at_ms < 0
            || state.updated_at_ms < state.created_at_ms
            || state.updated_at_ms > 253402300799999
            || state.version == 0
            || state.version > 2147483647
            || validation::text(&state.display_name, 1, 100, true).is_err()
            || state
                .display_name
                .chars()
                .any(|c| (c as u32) < 32 || c == '\u{7f}')
        {
            return Err(invalid());
        }
        Ok(Self { state })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.clone()
    }
    fn transition(&self, status: Status, expected: u32, at: i64) -> Result<Self, Failure> {
        let conflict = |code| {
            Failure::new(ErrorKind::Conflict, "principal transition refused").with_type(code)
        };
        if expected != self.state.version {
            return Err(conflict("identity.version_conflict"));
        }
        if status == self.state.status {
            return Err(conflict("identity.state_conflict"));
        }
        if at < self.state.updated_at_ms || at > 253402300799999 {
            return Err(invalid());
        }
        if self.state.version == 2147483647 {
            return Err(conflict("identity.version_exhausted"));
        }
        let mut state = self.state.clone();
        state.status = status;
        state.version += 1;
        state.updated_at_ms = at;
        Ok(Self { state })
    }
    pub fn suspend(&self, expected: u32, at: i64) -> Result<Self, Failure> {
        self.transition(Status::Suspended, expected, at)
    }
    pub fn activate(&self, expected: u32, at: i64) -> Result<Self, Failure> {
        self.transition(Status::Active, expected, at)
    }
}
