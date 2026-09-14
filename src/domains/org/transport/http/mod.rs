//! Organization HTTP projection; see CONTRACT.md.
#![doc = include_str!("CONTRACT.md")]

use crate::domains::org::app::{
    AcceptInvitation, AcceptInvitationInput, ChangeMembershipRole, ChangeMembershipRoleInput,
    Clock as OrgClock, CreateOrganization, CreateOrganizationInput, IdSource as OrgIdSource,
    InvitationStore, InviteMember, InviteMemberInput, ListOrganizations, OrganizationReader,
    OrganizationStore, PrincipalEligibility,
};
use crate::shared::{
    errors::{Failure, Kind},
    http::axum::{Admit, Refusal, RequestContext, RequestFailure, Response, Route, Work},
    id::Id,
};
use serde::de::{Deserializer, MapAccess, Visitor};
use std::{any::Any, collections::BTreeMap, sync::Arc};

pub type PrincipalId =
    Arc<dyn Fn(Option<&(dyn Any + Send + Sync)>) -> Result<Id, Failure> + Send + Sync>;
pub struct Config {
    pub admission: Admit,
    pub read_admission: Admit,
    pub principal_id: PrincipalId,
}
pub struct Transport<P> {
    config: Config,
    create: Arc<CreateOrganization<P>>,
    list: Arc<ListOrganizations<P>>,
    invite: Arc<InviteMember<P>>,
    accept: Arc<AcceptInvitation<P>>,
    role: Arc<ChangeMembershipRole<P>>,
}

fn invalid_body() -> Failure {
    Failure::new(Kind::Invalid, "invalid request body").with_type("http.invalid_body")
}
fn refused(error: Failure) -> RequestFailure {
    RequestFailure::Refused(Refusal::new(error).with_header("Cache-Control", "no-store"))
}

struct UniqueObject;
impl<'de> Visitor<'de> for UniqueObject {
    type Value = BTreeMap<String, serde_json::Value>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON object without duplicate members")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut result = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value::<serde_json::Value>()?;
            if result.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate member"));
            }
        }
        Ok(result)
    }
}
fn decode_name(body: &[u8]) -> Result<String, RequestFailure> {
    if body.is_empty() {
        return Err(refused(invalid_body()));
    }
    let mut de = serde_json::Deserializer::from_slice(body);
    let object = de
        .deserialize_map(UniqueObject)
        .map_err(|_| refused(invalid_body()))?;
    de.end().map_err(|_| refused(invalid_body()))?;
    if object.len() != 1 {
        return Err(refused(invalid_body()));
    }
    match object.get("name") {
        Some(serde_json::Value::String(name)) if !name.is_empty() => Ok(name.clone()),
        _ => Err(refused(invalid_body())),
    }
}
fn decode_fields(
    body: &[u8],
    expected: &[&str],
) -> Result<BTreeMap<String, String>, RequestFailure> {
    if body.is_empty() {
        return Err(refused(invalid_body()));
    }
    let mut de = serde_json::Deserializer::from_slice(body);
    let object = de
        .deserialize_map(UniqueObject)
        .map_err(|_| refused(invalid_body()))?;
    de.end().map_err(|_| refused(invalid_body()))?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(refused(invalid_body()));
    }
    object
        .into_iter()
        .map(|(key, value)| match value {
            serde_json::Value::String(value) if !value.is_empty() => Ok((key, value)),
            _ => Err(refused(invalid_body())),
        })
        .collect()
}
fn parse_id_field(fields: &BTreeMap<String, String>, name: &str) -> Result<Id, RequestFailure> {
    Id::parse(fields.get(name).map(String::as_str).unwrap_or_default())
        .map_err(|_| refused(invalid_body()))
}
fn parse_role_field(
    fields: &BTreeMap<String, String>,
) -> Result<crate::domains::org::domain::Role, RequestFailure> {
    match fields.get("role").map(String::as_str) {
        Some("admin") => Ok(crate::domains::org::domain::Role::Admin),
        Some("member") => Ok(crate::domains::org::domain::Role::Member),
        _ => Err(refused(invalid_body())),
    }
}
fn invitation_projection(s: crate::domains::org::domain::InvitationSnapshot) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": s.id.to_string(), "organization_id": s.organization_id.to_string(),
        "inviter_principal_id": s.inviter_principal_id.to_string(), "invitee_principal_id": s.invitee_principal_id.to_string(),
        "role": match s.role { crate::domains::org::domain::Role::Admin => "admin", crate::domains::org::domain::Role::Member => "member", crate::domains::org::domain::Role::Owner => "owner" },
        "status": match s.status { crate::domains::org::domain::InvitationStatus::Pending => "pending", crate::domains::org::domain::InvitationStatus::Accepted => "accepted", crate::domains::org::domain::InvitationStatus::Revoked => "revoked" },
        "created_at_ms": s.created_at_ms, "expires_at_ms": s.expires_at_ms,
    });
    if let Some(at) = s.resolved_at_ms {
        value["resolved_at_ms"] = serde_json::json!(at);
    }
    value
}
fn membership_projection(s: crate::domains::org::domain::MembershipSnapshot) -> serde_json::Value {
    serde_json::json!({
        "id": s.id.to_string(), "organization_id": s.organization_id.to_string(), "principal_id": s.principal_id.to_string(),
        "role": match s.role { crate::domains::org::domain::Role::Owner => "owner", crate::domains::org::domain::Role::Admin => "admin", crate::domains::org::domain::Role::Member => "member" },
        "created_at_ms": s.created_at_ms,
    })
}

impl<P> Transport<P>
where
    P: OrgClock
        + OrgIdSource
        + PrincipalEligibility
        + OrganizationStore
        + OrganizationReader
        + InvitationStore
        + Send
        + Sync
        + 'static,
{
    pub fn new(
        config: Config,
        create: Arc<CreateOrganization<P>>,
        list: Arc<ListOrganizations<P>>,
        invite: Arc<InviteMember<P>>,
        accept: Arc<AcceptInvitation<P>>,
        role: Arc<ChangeMembershipRole<P>>,
    ) -> Result<Self, Failure> {
        Ok(Self {
            config,
            create,
            list,
            invite,
            accept,
            role,
        })
    }
    async fn create_route(&self, context: RequestContext) -> Result<Response, RequestFailure> {
        let name = decode_name(&context.request.body)?;
        let principal =
            (self.config.principal_id)(context.request.admitted.as_deref()).map_err(refused)?;
        let result = self
            .create
            .execute(CreateOrganizationInput {
                name,
                owner_principal_id: principal,
            })
            .await
            .map_err(refused)?;
        Ok(Response::json(serde_json::json!({
            "organization": {
                "id": result.organization.id.to_string(),
                "name": result.organization.name,
                "created_at_ms": result.organization.created_at_ms,
            },
            "owner": {
                "id": result.owner.id.to_string(),
                "organization_id": result.owner.organization_id.to_string(),
                "principal_id": result.owner.principal_id.to_string(),
                "role": "owner",
                "created_at_ms": result.owner.created_at_ms,
            },
        }))
        .with_status(201)
        .with_header("Cache-Control", "no-store"))
    }
    async fn list_route(&self, context: RequestContext) -> Result<Response, RequestFailure> {
        let limit = query_limit(&context.request.query).map_err(refused)?;
        let principal =
            (self.config.principal_id)(context.request.admitted.as_deref()).map_err(refused)?;
        let items = self.list.execute(principal, limit).await.map_err(refused)?;
        let organizations: Vec<_> = items
            .into_iter()
            .map(|item| {
                serde_json::json!({
                    "organization": {
                        "id": item.organization.id.to_string(),
                        "name": item.organization.name,
                        "created_at_ms": item.organization.created_at_ms,
                    },
                    "membership": {
                        "id": item.membership.id.to_string(),
                        "organization_id": item.membership.organization_id.to_string(),
                        "principal_id": item.membership.principal_id.to_string(),
                        "role": match item.membership.role {
                            crate::domains::org::domain::Role::Owner => "owner",
                            crate::domains::org::domain::Role::Admin => "admin",
                            crate::domains::org::domain::Role::Member => "member",
                        },
                        "created_at_ms": item.membership.created_at_ms,
                    },
                })
            })
            .collect();
        Ok(Response::json(serde_json::json!({
            "organizations": organizations,
            "limit": limit,
        }))
        .with_header("Cache-Control", "no-store"))
    }
    async fn invite_route(&self, context: RequestContext) -> Result<Response, RequestFailure> {
        let fields = decode_fields(
            &context.request.body,
            &["organization_id", "principal_id", "role"],
        )?;
        let principal =
            (self.config.principal_id)(context.request.admitted.as_deref()).map_err(refused)?;
        let result = self
            .invite
            .execute(InviteMemberInput {
                organization_id: parse_id_field(&fields, "organization_id")?,
                inviter_id: principal,
                invitee_id: parse_id_field(&fields, "principal_id")?,
                role: parse_role_field(&fields)?,
            })
            .await
            .map_err(refused)?;
        Ok(Response::json(
            serde_json::json!({"invitation": invitation_projection(result.invitation)}),
        )
        .with_status(201)
        .with_header("Cache-Control", "no-store"))
    }
    async fn accept_route(&self, context: RequestContext) -> Result<Response, RequestFailure> {
        let fields = decode_fields(&context.request.body, &["invitation_id"])?;
        let principal =
            (self.config.principal_id)(context.request.admitted.as_deref()).map_err(refused)?;
        let result = self
            .accept
            .execute(AcceptInvitationInput {
                invitation_id: parse_id_field(&fields, "invitation_id")?,
                invitee_id: principal,
            })
            .await
            .map_err(refused)?;
        Ok(Response::json(serde_json::json!({"invitation": invitation_projection(result.invitation), "membership": membership_projection(result.membership)})).with_status(200).with_header("Cache-Control", "no-store"))
    }
    async fn role_route(&self, context: RequestContext) -> Result<Response, RequestFailure> {
        let fields = decode_fields(
            &context.request.body,
            &["organization_id", "principal_id", "role"],
        )?;
        let principal =
            (self.config.principal_id)(context.request.admitted.as_deref()).map_err(refused)?;
        let result = self
            .role
            .execute(ChangeMembershipRoleInput {
                organization_id: parse_id_field(&fields, "organization_id")?,
                actor_id: principal,
                target_id: parse_id_field(&fields, "principal_id")?,
                role: parse_role_field(&fields)?,
            })
            .await
            .map_err(refused)?;
        Ok(Response::json(
            serde_json::json!({"membership": membership_projection(result.membership)}),
        )
        .with_status(200)
        .with_header("Cache-Control", "no-store"))
    }
    pub fn routes(self: &Arc<Self>) -> Vec<Route> {
        let read = self.clone();
        let write = self.clone();
        let invite = self.clone();
        let accept = self.clone();
        let role = self.clone();
        vec![
            Route {
                path: "/v1/org/organizations".into(),
                method: "GET".into(),
                operation: "org.http.list".into(),
                admission: Some(read.config.read_admission.clone()),
                handler: Arc::new(move |context| {
                    let this = read.clone();
                    Box::pin(async move { this.list_route(context).await }) as Work
                }),
            },
            Route {
                path: "/v1/org/organizations".into(),
                method: "POST".into(),
                operation: "org.http.create".into(),
                admission: Some(write.config.admission.clone()),
                handler: Arc::new(move |context| {
                    let this = write.clone();
                    Box::pin(async move { this.create_route(context).await }) as Work
                }),
            },
            Route {
                path: "/v1/org/invitations".into(),
                method: "POST".into(),
                operation: "org.http.invite".into(),
                admission: Some(invite.config.admission.clone()),
                handler: Arc::new(move |context| {
                    let this = invite.clone();
                    Box::pin(async move { this.invite_route(context).await }) as Work
                }),
            },
            Route {
                path: "/v1/org/invitations/accept".into(),
                method: "POST".into(),
                operation: "org.http.invitation_accept".into(),
                admission: Some(accept.config.admission.clone()),
                handler: Arc::new(move |context| {
                    let this = accept.clone();
                    Box::pin(async move { this.accept_route(context).await }) as Work
                }),
            },
            Route {
                path: "/v1/org/memberships/role".into(),
                method: "POST".into(),
                operation: "org.http.role_change".into(),
                admission: Some(role.config.admission.clone()),
                handler: Arc::new(move |context| {
                    let this = role.clone();
                    Box::pin(async move { this.role_route(context).await }) as Work
                }),
            },
        ]
    }
}

fn query_limit(raw: &str) -> Result<i64, Failure> {
    let mut limit = 50;
    let mut seen = false;
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        if key == "limit" {
            if seen {
                return Err(invalid_query());
            }
            seen = true;
            limit = value.parse().map_err(|_| invalid_query())?;
        }
    }
    if !(1..=100).contains(&limit) {
        return Err(invalid_query());
    }
    Ok(limit)
}
fn invalid_query() -> Failure {
    Failure::new(Kind::Invalid, "invalid organization query").with_type("org.invalid_query")
}
