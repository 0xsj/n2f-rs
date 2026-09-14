use super::{MAX_VERSION, version_in_range};
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
};
/// Principal security epoch (A04). Password replacement and security-wide
/// revocation increment it; sessions carrying an older epoch are refused elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthState {
    principal_id: Id,
    epoch: u32,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid auth state").with_type("identity.auth_state_invalid")
}
fn conflict(code: &str) -> Failure {
    Failure::new(Kind::Conflict, "auth state transition refused").with_type(code)
}
impl AuthState {
    pub fn new(principal_id: Id, epoch: u32) -> Result<Self, Failure> {
        if !version_in_range(epoch) {
            return Err(invalid());
        }
        Ok(Self {
            principal_id,
            epoch,
        })
    }
    pub fn principal_id(&self) -> Id {
        self.principal_id
    }
    pub fn epoch(&self) -> u32 {
        self.epoch
    }
    /// Returns the next epoch as a new value; the stale guard is local only, and a
    /// persistence adapter must still enforce the expected epoch in its write.
    pub fn invalidate(&self, expected: u32) -> Result<Self, Failure> {
        if expected != self.epoch {
            return Err(conflict("identity.version_conflict"));
        }
        if self.epoch == MAX_VERSION {
            return Err(conflict("identity.version_exhausted"));
        }
        Ok(Self {
            principal_id: self.principal_id,
            epoch: self.epoch + 1,
        })
    }
}
