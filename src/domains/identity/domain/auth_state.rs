use super::auth_spec_stub::pending;
use crate::shared::{errors::Failure, id::Id};
pub struct AuthState {
    principal_id: Id,
    epoch: u32,
}
impl AuthState {
    pub fn new(_principal_id: Id, _epoch: u32) -> Result<Self, Failure> {
        Err(pending())
    }
    pub fn principal_id(&self) -> Id {
        self.principal_id
    }
    pub fn epoch(&self) -> u32 {
        self.epoch
    }
    pub fn invalidate(&self, _expected: u32) -> Result<Self, Failure> {
        Err(pending())
    }
}
