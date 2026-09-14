//! Organization application operations against org-owned ports.
#![doc = include_str!("app/CONTRACT.md")]

use crate::{
    domains::org::domain::{
        Invitation, InvitationSnapshot, Membership, MembershipSnapshot, Organization,
        OrganizationSnapshot, Role,
    },
    shared::{
        errors::{Failure, Kind},
        id::Id,
    },
};
use std::{
    future::Future,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_TIME_MS: i64 = 253402300799999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Eligibility {
    pub eligible: bool,
}

/// Root translates identity's deliberate application surface into this org-owned port.
pub trait PrincipalEligibility {
    fn check_active(
        &self,
        principal_id: Id,
    ) -> impl Future<Output = Result<Eligibility, Failure>> + Send;
}
pub trait Clock {
    fn now(&self) -> SystemTime;
}
pub trait IdSource {
    fn new_id(&self) -> Result<Id, Failure>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOrganizationRecord {
    pub organization: OrganizationSnapshot,
    pub owner: MembershipSnapshot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateOutcome {
    Created,
    AlreadyExists,
}
pub trait OrganizationStore {
    /// The adapter owns the transaction containing the organization and owner.
    fn create_organization(
        &self,
        record: CreateOrganizationRecord,
    ) -> impl Future<Output = Result<CreateOutcome, Failure>> + Send;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationMembership {
    pub organization: OrganizationSnapshot,
    pub membership: MembershipSnapshot,
}
pub trait OrganizationReader {
    /// Returns only organizations where the principal has a membership.
    fn list_for_principal(
        &self,
        principal_id: Id,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<OrganizationMembership>, Failure>> + Send;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationRecord {
    pub invitation: InvitationSnapshot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationCreateOutcome {
    Created,
    AlreadyExists,
    MemberExists,
    Unauthorized,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationAcceptOutcome {
    Accepted,
    AlreadyAccepted,
    Expired,
    NotFound,
    NotForPrincipal,
    MemberExists,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleChangeOutcome {
    Changed,
    Unchanged,
    TargetNotFound,
    Unauthorized,
    OwnerTarget,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptInvitationInput {
    pub invitation_id: Id,
    pub invitee_id: Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeMembershipRoleInput {
    pub organization_id: Id,
    pub actor_id: Id,
    pub target_id: Id,
    pub role: Role,
}
pub trait InvitationStore {
    fn create_invitation(
        &self,
        record: InvitationRecord,
    ) -> impl Future<Output = Result<InvitationCreateOutcome, Failure>> + Send;
    fn accept_invitation(
        &self,
        input: AcceptInvitationInput,
        now_ms: i64,
        membership_id: Id,
    ) -> impl Future<
        Output = Result<
            (
                InvitationAcceptOutcome,
                Option<InvitationRecord>,
                Option<MembershipSnapshot>,
            ),
            Failure,
        >,
    > + Send;
    fn change_membership_role(
        &self,
        input: ChangeMembershipRoleInput,
    ) -> impl Future<Output = Result<(RoleChangeOutcome, Option<MembershipSnapshot>), Failure>> + Send;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOrganizationInput {
    pub name: String,
    pub owner_principal_id: Id,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOrganizationResult {
    pub organization: OrganizationSnapshot,
    pub owner: MembershipSnapshot,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteMemberInput {
    pub organization_id: Id,
    pub inviter_id: Id,
    pub invitee_id: Id,
    pub role: Role,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteMemberResult {
    pub invitation: InvitationSnapshot,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptInvitationResult {
    pub invitation: InvitationSnapshot,
    pub membership: MembershipSnapshot,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeMembershipRoleResult {
    pub membership: MembershipSnapshot,
}
pub struct ListOrganizations<P> {
    ports: P,
}
pub struct CreateOrganization<P> {
    ports: P,
}
pub struct InviteMember<P> {
    ports: P,
}
pub struct AcceptInvitation<P> {
    ports: P,
}
pub struct ChangeMembershipRole<P> {
    ports: P,
}

fn ineligible() -> Failure {
    Failure::new(Kind::Forbidden, "organization owner is not eligible")
        .with_type("org.principal_ineligible")
}
fn dependency(source: Option<Failure>) -> Failure {
    let failure = Failure::new(Kind::Unavailable, "organization creation dependency failed")
        .with_type("org.create_dependency_failed");
    match source {
        Some(cause) => failure.with_source(cause),
        None => failure,
    }
}
fn exists() -> Failure {
    Failure::new(Kind::Conflict, "organization already exists").with_type("org.organization_exists")
}
fn membership_invalid_input() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization membership input")
        .with_type("org.membership_invalid_input")
}
fn invitee_ineligible() -> Failure {
    Failure::new(Kind::Forbidden, "organization invitee is not eligible")
        .with_type("org.invitee_ineligible")
}
fn invitation_exists() -> Failure {
    Failure::new(Kind::Conflict, "organization invitation already exists")
        .with_type("org.invitation_exists")
}
fn member_exists() -> Failure {
    Failure::new(Kind::Conflict, "organization member already exists")
        .with_type("org.member_exists")
}
fn membership_forbidden() -> Failure {
    Failure::new(
        Kind::Forbidden,
        "organization membership action is not permitted",
    )
    .with_type("org.membership_forbidden")
}
fn invitation_not_found() -> Failure {
    Failure::new(Kind::NotFound, "organization invitation not found")
        .with_type("org.invitation_not_found")
}
fn invitation_principal_mismatch() -> Failure {
    Failure::new(
        Kind::Forbidden,
        "organization invitation is not for this principal",
    )
    .with_type("org.invitation_principal_mismatch")
}
fn invitation_expired() -> Failure {
    Failure::new(Kind::Conflict, "organization invitation expired")
        .with_type("org.invitation_expired")
}
fn invitation_already_accepted() -> Failure {
    Failure::new(Kind::Conflict, "organization invitation already accepted")
        .with_type("org.invitation_already_accepted")
}
fn role_unchanged() -> Failure {
    Failure::new(Kind::Conflict, "organization membership role is unchanged")
        .with_type("org.role_unchanged")
}
fn membership_not_found() -> Failure {
    Failure::new(Kind::NotFound, "organization membership not found")
        .with_type("org.membership_not_found")
}
fn role_transition_invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization role transition")
        .with_type("org.role_transition_invalid")
}
fn membership_dependency() -> Failure {
    Failure::new(
        Kind::Unavailable,
        "organization membership dependency failed",
    )
    .with_type("org.membership_dependency_failed")
}

impl<P> CreateOrganization<P> {
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
}

impl<P> ListOrganizations<P> {
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
}

impl<P> InviteMember<P> {
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
}
impl<P> AcceptInvitation<P> {
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
}
impl<P> ChangeMembershipRole<P> {
    pub fn new(ports: P) -> Result<Self, Failure> {
        Ok(Self { ports })
    }
}

impl<P> ListOrganizations<P>
where
    P: OrganizationReader + Send + Sync,
{
    pub async fn execute(
        &self,
        principal_id: Id,
        limit: i64,
    ) -> Result<Vec<OrganizationMembership>, Failure> {
        if principal_id.is_zero() || !(1..=100).contains(&limit) {
            return Err(
                Failure::new(Kind::Invalid, "invalid organization list input")
                    .with_type("org.list_invalid"),
            );
        }
        self.ports.list_for_principal(principal_id, limit).await
    }
}

impl<P> CreateOrganization<P>
where
    P: Clock + IdSource + PrincipalEligibility + OrganizationStore + Send + Sync,
{
    fn now(&self) -> Result<i64, Failure> {
        self.ports
            .now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .filter(|ms| (0..=MAX_TIME_MS).contains(ms))
            .ok_or_else(|| dependency(None))
    }
    fn new_id(&self) -> Result<Id, Failure> {
        let id = self
            .ports
            .new_id()
            .map_err(|failure| dependency(Some(failure)))?;
        Ok(id)
    }
    pub async fn execute(
        &self,
        input: CreateOrganizationInput,
    ) -> Result<CreateOrganizationResult, Failure> {
        let eligibility = self.ports.check_active(input.owner_principal_id).await?;
        if !eligibility.eligible {
            return Err(ineligible());
        }
        let now = self.now()?;
        let organization_id = self.new_id()?;
        let membership_id = self.new_id()?;
        let organization = Organization::create(organization_id, input.name, now)?;
        let owner = Membership::owner(
            membership_id,
            organization_id,
            input.owner_principal_id,
            now,
        )?;
        let record = CreateOrganizationRecord {
            organization: organization.snapshot(),
            owner: owner.snapshot(),
        };
        match self.ports.create_organization(record.clone()).await? {
            CreateOutcome::Created => Ok(CreateOrganizationResult {
                organization: record.organization,
                owner: record.owner,
            }),
            CreateOutcome::AlreadyExists => Err(exists()),
        }
    }
}

const INVITATION_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
fn now_ms<P: Clock>(ports: &P) -> Result<i64, Failure> {
    ports
        .now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .filter(|ms| (0..=MAX_TIME_MS).contains(ms))
        .ok_or_else(membership_dependency)
}
fn new_id<P: IdSource>(ports: &P) -> Result<Id, Failure> {
    ports.new_id().map_err(|_| membership_dependency())
}
fn invite_role(role: Role) -> bool {
    matches!(role, Role::Admin | Role::Member)
}

impl<P> InviteMember<P>
where
    P: Clock + IdSource + PrincipalEligibility + InvitationStore + Send + Sync,
{
    pub async fn execute(&self, input: InviteMemberInput) -> Result<InviteMemberResult, Failure> {
        if input.organization_id.is_zero()
            || input.inviter_id.is_zero()
            || input.invitee_id.is_zero()
            || input.inviter_id == input.invitee_id
            || !invite_role(input.role)
        {
            return Err(membership_invalid_input());
        }
        if !self.ports.check_active(input.invitee_id).await?.eligible {
            return Err(invitee_ineligible());
        }
        let now = now_ms(&self.ports)?;
        let expires = now
            .checked_add(INVITATION_TTL_MS)
            .filter(|v| *v <= MAX_TIME_MS)
            .ok_or_else(membership_dependency)?;
        let invitation = Invitation::create(
            new_id(&self.ports)?,
            input.organization_id,
            input.inviter_id,
            input.invitee_id,
            input.role,
            now,
            expires,
        )?;
        match self
            .ports
            .create_invitation(InvitationRecord {
                invitation: invitation.snapshot(),
            })
            .await?
        {
            InvitationCreateOutcome::Created => Ok(InviteMemberResult {
                invitation: invitation.snapshot(),
            }),
            InvitationCreateOutcome::AlreadyExists => Err(invitation_exists()),
            InvitationCreateOutcome::MemberExists => Err(member_exists()),
            InvitationCreateOutcome::Unauthorized => Err(membership_forbidden()),
        }
    }
}

impl<P> AcceptInvitation<P>
where
    P: Clock + IdSource + InvitationStore + Send + Sync,
{
    pub async fn execute(
        &self,
        input: AcceptInvitationInput,
    ) -> Result<AcceptInvitationResult, Failure> {
        if input.invitation_id.is_zero() || input.invitee_id.is_zero() {
            return Err(membership_invalid_input());
        }
        let outcome = self
            .ports
            .accept_invitation(input, now_ms(&self.ports)?, new_id(&self.ports)?)
            .await?;
        match outcome.0 {
            InvitationAcceptOutcome::Accepted => {
                let (Some(invitation), Some(membership)) = (outcome.1, outcome.2) else {
                    return Err(membership_dependency());
                };
                Ok(AcceptInvitationResult {
                    invitation: invitation.invitation,
                    membership,
                })
            }
            InvitationAcceptOutcome::AlreadyAccepted => Err(invitation_already_accepted()),
            InvitationAcceptOutcome::Expired => Err(invitation_expired()),
            InvitationAcceptOutcome::NotFound => Err(invitation_not_found()),
            InvitationAcceptOutcome::NotForPrincipal => Err(invitation_principal_mismatch()),
            InvitationAcceptOutcome::MemberExists => Err(member_exists()),
        }
    }
}

impl<P> ChangeMembershipRole<P>
where
    P: InvitationStore + Send + Sync,
{
    pub async fn execute(
        &self,
        input: ChangeMembershipRoleInput,
    ) -> Result<ChangeMembershipRoleResult, Failure> {
        if input.organization_id.is_zero()
            || input.actor_id.is_zero()
            || input.target_id.is_zero()
            || input.actor_id == input.target_id
            || !invite_role(input.role)
        {
            return Err(role_transition_invalid());
        }
        match self.ports.change_membership_role(input).await? {
            (RoleChangeOutcome::Changed, Some(membership)) => {
                Ok(ChangeMembershipRoleResult { membership })
            }
            (RoleChangeOutcome::Unchanged, _) => Err(role_unchanged()),
            (RoleChangeOutcome::TargetNotFound, _) => Err(membership_not_found()),
            (RoleChangeOutcome::Unauthorized, _) => Err(membership_forbidden()),
            (RoleChangeOutcome::OwnerTarget, _) => Err(role_transition_invalid()),
            (RoleChangeOutcome::Changed, None) => Err(membership_dependency()),
        }
    }
}
