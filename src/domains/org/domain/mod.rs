#![doc = include_str!("CONTRACT.md")]

use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
    validation,
};

const MAX_TIME_MS: i64 = 253402300799999;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationSnapshot {
    pub id: Id,
    pub name: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct Organization {
    state: OrganizationSnapshot,
}

fn invalid_organization() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization").with_type("org.organization_invalid")
}
fn valid_name(name: &str) -> bool {
    validation::text(name, 1, 100, true).is_ok()
        && !name.chars().any(|c| (c as u32) < 32 || c == '\u{7f}')
}
fn valid_organization(s: &OrganizationSnapshot) -> bool {
    !s.id.is_zero() && valid_name(&s.name) && (0..=MAX_TIME_MS).contains(&s.created_at_ms)
}
impl Organization {
    pub fn create(id: Id, name: String, at: i64) -> Result<Self, Failure> {
        Self::restore(OrganizationSnapshot {
            id,
            name,
            created_at_ms: at,
        })
    }
    pub fn restore(state: OrganizationSnapshot) -> Result<Self, Failure> {
        if !valid_organization(&state) {
            return Err(invalid_organization());
        }
        Ok(Self { state })
    }
    pub fn snapshot(&self) -> OrganizationSnapshot {
        self.state.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Member,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipSnapshot {
    pub id: Id,
    pub organization_id: Id,
    pub principal_id: Id,
    pub role: Role,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct Membership {
    state: MembershipSnapshot,
}

fn invalid_membership() -> Failure {
    Failure::new(Kind::Invalid, "invalid membership").with_type("org.membership_invalid")
}
fn valid_membership(s: &MembershipSnapshot) -> bool {
    !s.id.is_zero()
        && !s.organization_id.is_zero()
        && !s.principal_id.is_zero()
        && (0..=MAX_TIME_MS).contains(&s.created_at_ms)
}
impl Membership {
    pub fn create(
        id: Id,
        organization_id: Id,
        principal_id: Id,
        role: Role,
        at: i64,
    ) -> Result<Self, Failure> {
        let state = MembershipSnapshot {
            id,
            organization_id,
            principal_id,
            role,
            created_at_ms: at,
        };
        if !valid_membership(&state) {
            return Err(invalid_membership());
        }
        Ok(Self { state })
    }
    pub fn owner(id: Id, organization_id: Id, principal_id: Id, at: i64) -> Result<Self, Failure> {
        Self::create(id, organization_id, principal_id, Role::Owner, at)
    }
    pub fn restore(state: MembershipSnapshot) -> Result<Self, Failure> {
        if !valid_membership(&state) {
            return Err(invalid_membership());
        }
        Ok(Self { state })
    }
    pub fn snapshot(&self) -> MembershipSnapshot {
        self.state.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationSnapshot {
    pub id: Id,
    pub organization_id: Id,
    pub inviter_principal_id: Id,
    pub invitee_principal_id: Id,
    pub role: Role,
    pub status: InvitationStatus,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Invitation {
    state: InvitationSnapshot,
}

fn invalid_role() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization role").with_type("org.role_invalid")
}
fn invalid_invitation() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization invitation")
        .with_type("org.invitation_invalid")
}
fn invalid_invitation_state() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization invitation state")
        .with_type("org.invitation_state_invalid")
}
fn valid_invitation_role(role: Role) -> bool {
    matches!(role, Role::Admin | Role::Member)
}
fn valid_invitation(state: &InvitationSnapshot) -> bool {
    !state.id.is_zero()
        && !state.organization_id.is_zero()
        && !state.inviter_principal_id.is_zero()
        && !state.invitee_principal_id.is_zero()
        && state.inviter_principal_id != state.invitee_principal_id
        && valid_invitation_role(state.role)
        && state.created_at_ms >= 0
        && state.created_at_ms <= MAX_TIME_MS
        && state.expires_at_ms > state.created_at_ms
        && state.expires_at_ms <= MAX_TIME_MS
        && match (state.status, state.resolved_at_ms) {
            (InvitationStatus::Pending, None) => true,
            (InvitationStatus::Accepted | InvitationStatus::Revoked, Some(at)) => {
                at >= state.created_at_ms && at <= MAX_TIME_MS
            }
            _ => false,
        }
}
impl Invitation {
    pub fn create(
        id: Id,
        organization_id: Id,
        inviter_principal_id: Id,
        invitee_principal_id: Id,
        role: Role,
        created_at_ms: i64,
        expires_at_ms: i64,
    ) -> Result<Self, Failure> {
        Self::restore(InvitationSnapshot {
            id,
            organization_id,
            inviter_principal_id,
            invitee_principal_id,
            role,
            status: InvitationStatus::Pending,
            created_at_ms,
            expires_at_ms,
            resolved_at_ms: None,
        })
    }
    pub fn restore(state: InvitationSnapshot) -> Result<Self, Failure> {
        if !valid_invitation_role(state.role) {
            return Err(invalid_role());
        }
        if !valid_invitation(&state) {
            return Err(
                if matches!(
                    state.status,
                    InvitationStatus::Pending
                        | InvitationStatus::Accepted
                        | InvitationStatus::Revoked
                ) {
                    invalid_invitation()
                } else {
                    invalid_invitation_state()
                },
            );
        }
        Ok(Self { state })
    }
    pub fn accept(&self, at: i64) -> Result<Self, Failure> {
        if self.state.status != InvitationStatus::Pending {
            return Err(invalid_invitation_state());
        }
        if at < self.state.created_at_ms || at >= self.state.expires_at_ms {
            return Err(
                Failure::new(Kind::Conflict, "organization invitation expired")
                    .with_type("org.invitation_expired"),
            );
        }
        Self::restore(InvitationSnapshot {
            status: InvitationStatus::Accepted,
            resolved_at_ms: Some(at),
            ..self.state.clone()
        })
    }
    pub fn revoke(&self, at: i64) -> Result<Self, Failure> {
        if self.state.status != InvitationStatus::Pending {
            return Err(invalid_invitation_state());
        }
        if at < self.state.created_at_ms || at > MAX_TIME_MS {
            return Err(invalid_invitation());
        }
        Self::restore(InvitationSnapshot {
            status: InvitationStatus::Revoked,
            resolved_at_ms: Some(at),
            ..self.state.clone()
        })
    }
    pub fn snapshot(&self) -> InvitationSnapshot {
        self.state.clone()
    }
}
