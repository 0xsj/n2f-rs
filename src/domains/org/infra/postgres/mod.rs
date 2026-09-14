//! Organization PostgreSQL persistence; see CONTRACT.md.
#![doc = include_str!("CONTRACT.md")]

use crate::{
    domains::org::{
        app::{
            AcceptInvitationInput, ChangeMembershipRoleInput, InvitationAcceptOutcome,
            InvitationCreateOutcome, InvitationRecord, InvitationStore, RoleChangeOutcome,
        },
        app::{CreateOrganizationRecord, CreateOutcome, OrganizationStore},
        app::{OrganizationMembership, OrganizationReader},
        domain::{
            Invitation, InvitationSnapshot, InvitationStatus, Membership, MembershipSnapshot,
            Organization, OrganizationSnapshot, Role,
        },
    },
    shared::{
        errors::{Classified, Failure, Kind},
        postgres::{Database, Migration, map},
    },
};
use sqlx::{PgConnection, Row};
use std::{future::Future, sync::Arc};

const ROLLBACK: &str = "org.store_rollback";

pub fn migration(version: i64) -> Migration {
    Migration {
        version,
        sql: include_str!("migrations/0006_org.sql").into(),
    }
}
pub fn invitation_migration(version: i64) -> Migration {
    Migration {
        version,
        sql: include_str!("migrations/0007_org_membership_workflow.sql").into(),
    }
}

pub struct Store {
    database: Arc<Database>,
}
impl Store {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }
}

fn invalid_record() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization store record")
        .with_type("org.store_invalid_record")
}
fn rollback() -> Failure {
    Failure::new(Kind::Conflict, "organization store outcome rolled back").with_type(ROLLBACK)
}
fn is_rollback(failure: &Failure) -> bool {
    failure.classification().and_then(|c| c.error_type) == Some(ROLLBACK)
}
fn valid_record(record: &CreateOrganizationRecord) -> bool {
    let Ok(organization) = Organization::restore(record.organization.clone()) else {
        return false;
    };
    let Ok(owner) = Membership::restore(record.owner.clone()) else {
        return false;
    };
    let snapshot = owner.snapshot();
    snapshot.organization_id == organization.snapshot().id && snapshot.role == Role::Owner
}

impl OrganizationStore for Store {
    fn create_organization(
        &self,
        record: CreateOrganizationRecord,
    ) -> impl Future<Output = Result<CreateOutcome, Failure>> + Send {
        let database = self.database.clone();
        async move {
            if !valid_record(&record) {
                return Err(invalid_record());
            }
            let result = database.transaction(move |tx| {
                Box::pin(async move {
                    let organization = sqlx::query("INSERT INTO public.n2f_org_organizations(id,name,created_at_ms) VALUES($1::uuid,$2,$3) ON CONFLICT(id) DO NOTHING")
                        .bind(record.organization.id.to_string())
                        .bind(record.organization.name)
                        .bind(record.organization.created_at_ms)
                        .execute(&mut *tx).await.map_err(map)?;
                    if organization.rows_affected() != 1 { return Err(rollback()); }
                    let owner = sqlx::query("INSERT INTO public.n2f_org_memberships(id,organization_id,principal_id,role,created_at_ms) VALUES($1::uuid,$2::uuid,$3::uuid,$4,$5) ON CONFLICT(id) DO NOTHING")
                        .bind(record.owner.id.to_string())
                        .bind(record.owner.organization_id.to_string())
                        .bind(record.owner.principal_id.to_string())
                        .bind(match record.owner.role { Role::Owner => "owner", Role::Admin => "admin", Role::Member => "member" })
                        .bind(record.owner.created_at_ms)
                        .execute(&mut *tx).await.map_err(map)?;
                    if owner.rows_affected() != 1 { return Err(rollback()); }
                    Ok(())
                })
            }).await;
            match result {
                Ok(()) => Ok(CreateOutcome::Created),
                Err(error) if is_rollback(&error) => Ok(CreateOutcome::AlreadyExists),
                Err(error) => Err(error),
            }
        }
    }
}

impl OrganizationReader for Store {
    fn list_for_principal(
        &self,
        principal_id: crate::shared::id::Id,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<OrganizationMembership>, Failure>> + Send {
        let database = self.database.clone();
        async move {
            if principal_id.is_zero() || !(1..=100).contains(&limit) {
                return Err(invalid_record());
            }
            database
                .transaction(move |tx| {
                    Box::pin(async move {
                        let rows = sqlx::query("SELECT o.id::text AS organization_id,o.name,o.created_at_ms AS organization_created_at_ms,m.id::text AS membership_id,m.organization_id::text AS membership_organization_id,m.principal_id::text AS membership_principal_id,m.role,m.created_at_ms AS membership_created_at_ms FROM public.n2f_org_memberships m JOIN public.n2f_org_organizations o ON o.id=m.organization_id WHERE m.principal_id=$1::uuid ORDER BY o.created_at_ms DESC,o.id DESC LIMIT $2")
                            .bind(principal_id.to_string())
                            .bind(limit)
                            .fetch_all(&mut *tx)
                            .await
                            .map_err(map)?;
                        let mut out = Vec::with_capacity(rows.len());
                        for row in rows {
                            let organization = Organization::restore(OrganizationSnapshot {
                                id: crate::shared::id::Id::parse(
                                    row.try_get::<String, _>("organization_id").map_err(map)?.as_str(),
                                )
                                .map_err(|_| invalid_record())?,
                                name: row.try_get("name").map_err(map)?,
                                created_at_ms: row.try_get("organization_created_at_ms").map_err(map)?,
                            })
                            .map_err(|_| invalid_record())?;
                            let membership = Membership::restore(MembershipSnapshot {
                                id: crate::shared::id::Id::parse(
                                    row.try_get::<String, _>("membership_id").map_err(map)?.as_str(),
                                )
                                .map_err(|_| invalid_record())?,
                                organization_id: crate::shared::id::Id::parse(
                                    row.try_get::<String, _>("membership_organization_id").map_err(map)?.as_str(),
                                )
                                .map_err(|_| invalid_record())?,
                                principal_id: crate::shared::id::Id::parse(
                                    row.try_get::<String, _>("membership_principal_id").map_err(map)?.as_str(),
                                )
                                .map_err(|_| invalid_record())?,
                                role: match row.try_get::<String, _>("role").map_err(map)?.as_str() {
                                    "owner" => Role::Owner,
                                    "admin" => Role::Admin,
                                    "member" => Role::Member,
                                    _ => return Err(invalid_record()),
                                },
                                created_at_ms: row.try_get("membership_created_at_ms").map_err(map)?,
                            })
                            .map_err(|_| invalid_record())?;
                            let organization = organization.snapshot();
                            let membership = membership.snapshot();
                            if membership.organization_id != organization.id
                                || membership.principal_id != principal_id
                            {
                                return Err(invalid_record());
                            }
                            out.push(OrganizationMembership { organization, membership });
                        }
                        Ok(out)
                    })
                })
                .await
        }
    }
}

fn invitation_role(raw: &str) -> Result<Role, Failure> {
    match raw {
        "admin" => Ok(Role::Admin),
        "member" => Ok(Role::Member),
        _ => Err(invalid_record()),
    }
}
fn invitation_status(raw: &str) -> Result<InvitationStatus, Failure> {
    match raw {
        "pending" => Ok(InvitationStatus::Pending),
        "accepted" => Ok(InvitationStatus::Accepted),
        "revoked" => Ok(InvitationStatus::Revoked),
        _ => Err(invalid_record()),
    }
}
fn invitation_out(row: &sqlx::postgres::PgRow) -> Result<InvitationSnapshot, Failure> {
    let value = InvitationSnapshot {
        id: crate::shared::id::Id::parse(row.try_get::<String, _>("id").map_err(map)?.as_str())
            .map_err(|_| invalid_record())?,
        organization_id: crate::shared::id::Id::parse(
            row.try_get::<String, _>("organization_id")
                .map_err(map)?
                .as_str(),
        )
        .map_err(|_| invalid_record())?,
        inviter_principal_id: crate::shared::id::Id::parse(
            row.try_get::<String, _>("inviter_principal_id")
                .map_err(map)?
                .as_str(),
        )
        .map_err(|_| invalid_record())?,
        invitee_principal_id: crate::shared::id::Id::parse(
            row.try_get::<String, _>("invitee_principal_id")
                .map_err(map)?
                .as_str(),
        )
        .map_err(|_| invalid_record())?,
        role: invitation_role(row.try_get::<String, _>("role").map_err(map)?.as_str())?,
        status: invitation_status(row.try_get::<String, _>("status").map_err(map)?.as_str())?,
        created_at_ms: row.try_get("created_at_ms").map_err(map)?,
        expires_at_ms: row.try_get("expires_at_ms").map_err(map)?,
        resolved_at_ms: row.try_get("resolved_at_ms").map_err(map)?,
    };
    Invitation::restore(value)
        .map(|v| v.snapshot())
        .map_err(|_| invalid_record())
}
async fn membership_role(
    tx: &mut PgConnection,
    organization_id: crate::shared::id::Id,
    principal_id: crate::shared::id::Id,
) -> Result<Option<Role>, Failure> {
    let row = sqlx::query("SELECT role FROM public.n2f_org_memberships WHERE organization_id=$1::uuid AND principal_id=$2::uuid FOR UPDATE")
        .bind(organization_id.to_string())
        .bind(principal_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map)?;
    row.map(
        |r| match r.try_get::<String, _>("role").map_err(map)?.as_str() {
            "owner" => Ok(Role::Owner),
            "admin" => Ok(Role::Admin),
            "member" => Ok(Role::Member),
            _ => Err(invalid_record()),
        },
    )
    .transpose()
}

impl InvitationStore for Store {
    fn create_invitation(
        &self,
        record: InvitationRecord,
    ) -> impl Future<Output = Result<InvitationCreateOutcome, Failure>> + Send {
        let database = self.database.clone();
        async move {
            if Invitation::restore(record.invitation.clone()).is_err() {
                return Err(invalid_record());
            }
            database.transaction(move |tx| {
                Box::pin(async move {
                    let invitation = record.invitation;
                    let actor = membership_role(&mut *tx, invitation.organization_id, invitation.inviter_principal_id).await?;
                    if !matches!(actor, Some(Role::Owner | Role::Admin)) || (actor == Some(Role::Admin) && invitation.role == Role::Admin) {
                        return Ok(InvitationCreateOutcome::Unauthorized);
                    }
                    let member: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.n2f_org_memberships WHERE organization_id=$1::uuid AND principal_id=$2::uuid)")
                        .bind(invitation.organization_id.to_string()).bind(invitation.invitee_principal_id.to_string()).fetch_one(&mut *tx).await.map_err(map)?;
                    if member { return Ok(InvitationCreateOutcome::MemberExists); }
                    let inserted = sqlx::query("INSERT INTO public.n2f_org_invitations(id,organization_id,inviter_principal_id,invitee_principal_id,role,status,created_at_ms,expires_at_ms,resolved_at_ms) VALUES($1::uuid,$2::uuid,$3::uuid,$4::uuid,$5,$6,$7,$8,$9) ON CONFLICT (organization_id,invitee_principal_id) WHERE status='pending' DO NOTHING")
                        .bind(invitation.id.to_string()).bind(invitation.organization_id.to_string()).bind(invitation.inviter_principal_id.to_string()).bind(invitation.invitee_principal_id.to_string())
                        .bind(match invitation.role { Role::Admin => "admin", Role::Member => "member", Role::Owner => "owner" })
                        .bind(match invitation.status { InvitationStatus::Pending => "pending", InvitationStatus::Accepted => "accepted", InvitationStatus::Revoked => "revoked" })
                        .bind(invitation.created_at_ms).bind(invitation.expires_at_ms).bind(invitation.resolved_at_ms)
                        .execute(&mut *tx).await.map_err(map)?;
                    Ok(if inserted.rows_affected() == 1 { InvitationCreateOutcome::Created } else { InvitationCreateOutcome::AlreadyExists })
                })
            }).await
        }
    }

    fn accept_invitation(
        &self,
        input: AcceptInvitationInput,
        now_ms: i64,
        membership_id: crate::shared::id::Id,
    ) -> impl Future<
        Output = Result<
            (
                InvitationAcceptOutcome,
                Option<InvitationRecord>,
                Option<MembershipSnapshot>,
            ),
            Failure,
        >,
    > + Send {
        let database = self.database.clone();
        async move {
            database.transaction(move |tx| {
                Box::pin(async move {
                    let row = sqlx::query("SELECT id::text,organization_id::text,inviter_principal_id::text,invitee_principal_id::text,role,status,created_at_ms,expires_at_ms,resolved_at_ms FROM public.n2f_org_invitations WHERE id=$1::uuid FOR UPDATE")
                        .bind(input.invitation_id.to_string()).fetch_optional(&mut *tx).await.map_err(map)?;
                    let Some(row) = row else { return Ok((InvitationAcceptOutcome::NotFound, None, None)); };
                    let invitation = invitation_out(&row)?;
                    if invitation.invitee_principal_id != input.invitee_id { return Ok((InvitationAcceptOutcome::NotForPrincipal, None, None)); }
                    if invitation.status == InvitationStatus::Accepted { return Ok((InvitationAcceptOutcome::AlreadyAccepted, None, None)); }
                    if invitation.status != InvitationStatus::Pending { return Ok((InvitationAcceptOutcome::Expired, None, None)); }
                    let domain = Invitation::restore(invitation.clone()).map_err(|_| invalid_record())?;
                    if now_ms >= invitation.expires_at_ms {
                        let revoked = domain.revoke(now_ms).map_err(|_| invalid_record())?;
                        sqlx::query("UPDATE public.n2f_org_invitations SET status='revoked',resolved_at_ms=$2 WHERE id=$1::uuid")
                            .bind(invitation.id.to_string()).bind(revoked.snapshot().resolved_at_ms).execute(&mut *tx).await.map_err(map)?;
                        return Ok((InvitationAcceptOutcome::Expired, None, None));
                    }
                    let member: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.n2f_org_memberships WHERE organization_id=$1::uuid AND principal_id=$2::uuid)")
                        .bind(invitation.organization_id.to_string()).bind(invitation.invitee_principal_id.to_string()).fetch_one(&mut *tx).await.map_err(map)?;
                    if member { return Ok((InvitationAcceptOutcome::MemberExists, None, None)); }
                    let membership = Membership::create(membership_id, invitation.organization_id, invitation.invitee_principal_id, invitation.role, now_ms)?;
                    let inserted = sqlx::query("INSERT INTO public.n2f_org_memberships(id,organization_id,principal_id,role,created_at_ms) VALUES($1::uuid,$2::uuid,$3::uuid,$4,$5) ON CONFLICT (organization_id,principal_id) DO NOTHING")
                        .bind(membership_id.to_string()).bind(invitation.organization_id.to_string()).bind(invitation.invitee_principal_id.to_string())
                        .bind(match invitation.role { Role::Admin => "admin", Role::Member => "member", Role::Owner => "owner" }).bind(now_ms)
                        .execute(&mut *tx).await.map_err(map)?;
                    if inserted.rows_affected() != 1 { return Ok((InvitationAcceptOutcome::MemberExists, None, None)); }
                    let accepted = domain.accept(now_ms).map_err(|_| invalid_record())?;
                    sqlx::query("UPDATE public.n2f_org_invitations SET status='accepted',resolved_at_ms=$2 WHERE id=$1::uuid")
                        .bind(invitation.id.to_string()).bind(accepted.snapshot().resolved_at_ms).execute(&mut *tx).await.map_err(map)?;
                    Ok((InvitationAcceptOutcome::Accepted, Some(InvitationRecord { invitation: accepted.snapshot() }), Some(membership.snapshot())))
                })
            }).await
        }
    }

    fn change_membership_role(
        &self,
        input: ChangeMembershipRoleInput,
    ) -> impl Future<Output = Result<(RoleChangeOutcome, Option<MembershipSnapshot>), Failure>> + Send
    {
        let database = self.database.clone();
        async move {
            database.transaction(move |tx| {
                Box::pin(async move {
                    let actor = membership_role(&mut *tx, input.organization_id, input.actor_id).await?;
                    if !matches!(actor, Some(Role::Owner | Role::Admin)) || (actor == Some(Role::Admin) && input.role == Role::Admin) {
                        return Ok((RoleChangeOutcome::Unauthorized, None));
                    }
                    let row = sqlx::query("SELECT id::text,organization_id::text,principal_id::text,role,created_at_ms FROM public.n2f_org_memberships WHERE organization_id=$1::uuid AND principal_id=$2::uuid FOR UPDATE")
                        .bind(input.organization_id.to_string()).bind(input.target_id.to_string()).fetch_optional(&mut *tx).await.map_err(map)?;
                    let Some(row) = row else { return Ok((RoleChangeOutcome::TargetNotFound, None)); };
                    let target_role = match row.try_get::<String, _>("role").map_err(map)?.as_str() {
                        "owner" => Role::Owner, "admin" => Role::Admin, "member" => Role::Member, _ => return Err(invalid_record()),
                    };
                    if target_role == Role::Owner { return Ok((RoleChangeOutcome::OwnerTarget, None)); }
                    if target_role == input.role { return Ok((RoleChangeOutcome::Unchanged, None)); }
                    let id = row.try_get::<String, _>("id").map_err(map)?;
                    let organization_id = row.try_get::<String, _>("organization_id").map_err(map)?;
                    let principal_id = row.try_get::<String, _>("principal_id").map_err(map)?;
                    sqlx::query("UPDATE public.n2f_org_memberships SET role=$2 WHERE id=$1::uuid")
                        .bind(&id).bind(match input.role { Role::Admin => "admin", Role::Member => "member", Role::Owner => "owner" }).execute(&mut *tx).await.map_err(map)?;
                    Ok((RoleChangeOutcome::Changed, Some(Membership::create(
                        crate::shared::id::Id::parse(&id).map_err(|_| invalid_record())?,
                        crate::shared::id::Id::parse(&organization_id).map_err(|_| invalid_record())?,
                        crate::shared::id::Id::parse(&principal_id).map_err(|_| invalid_record())?,
                        input.role, row.try_get("created_at_ms").map_err(map)?,
                    )?.snapshot())))
                })
            }).await
        }
    }
}
