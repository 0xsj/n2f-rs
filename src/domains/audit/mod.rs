//! Audit owns durable business facts and their authenticated read projection.
use crate::shared::{
    errors::{Failure, Kind},
    events::Envelope,
    http::axum::{Admit, RequestContext, RequestFailure, Response, Route},
    id::Id,
    postgres::{Database, Migration, map},
    provenance::{ActorKind, Origin},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, Row};
use std::sync::Arc;

pub fn migration(version: i64) -> Migration {
    Migration {
        version,
        sql: include_str!("migrations/0004_audit.sql").into(),
    }
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub kind: String,
    pub identity: String,
}
#[derive(Clone, Debug)]
pub struct Record {
    pub id: Id,
    pub event_id: Id,
    pub occurred_at_ms: i64,
    pub recorded_at_ms: i64,
    pub action: String,
    pub outcome: String,
    pub actor: Actor,
    pub subject_id: Id,
    pub tenant: Option<String>,
    pub work_id: Id,
    pub correlation_id: Id,
    pub causation_id: Option<Id>,
    pub origin: Option<String>,
    pub details: Value,
}
#[derive(Serialize)]
struct ViewActor {
    kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    identity: String,
}
#[derive(Serialize)]
struct View {
    id: String,
    event_id: String,
    occurred_at_ms: i64,
    recorded_at_ms: i64,
    action: String,
    outcome: String,
    actor: ViewActor,
    subject_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant: Option<String>,
    work_id: String,
    correlation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    causation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    details: Value,
}
pub struct Store {
    pub database: Arc<Database>,
}

fn invalid(code: &str) -> Failure {
    Failure::new(Kind::Invalid, "invalid audit record").with_type(format!("audit.{code}"))
}
impl Store {
    pub async fn insert(&self, tx: &mut PgConnection, r: &Record) -> Result<(), Failure> {
        if r.id != r.event_id
            || r.action.is_empty()
            || r.outcome != "succeeded"
            || r.occurred_at_ms < 0
            || r.recorded_at_ms < 0
            || r.subject_id.to_string().is_empty()
            || !r.details.is_object()
        {
            return Err(invalid("invalid_record"));
        }
        sqlx::query("INSERT INTO public.n2f_audit_records (event_id,occurred_at_ms,recorded_at_ms,action,outcome,actor_kind,actor_identity,subject_id,tenant,work_id,correlation_id,causation_id,origin,details) VALUES($1::uuid,$2,$3,$4,$5,$6,$7,$8::uuid,$9,$10::uuid,$11::uuid,$12::uuid,$13,$14) ON CONFLICT(event_id) DO NOTHING")
            .bind(r.event_id.to_string()).bind(r.occurred_at_ms).bind(r.recorded_at_ms).bind(&r.action)
            .bind(&r.outcome).bind(&r.actor.kind).bind(&r.actor.identity).bind(r.subject_id.to_string())
            .bind(&r.tenant).bind(r.work_id.to_string()).bind(r.correlation_id.to_string())
            .bind(r.causation_id.map(|v| v.to_string())).bind(&r.origin)
            .bind(serde_json::to_string(&r.details).map_err(|_| invalid("invalid_record"))?)
            .execute(&mut *tx).await.map_err(map)?;
        Ok(())
    }
    pub async fn list(&self, limit: i64) -> Result<Vec<Value>, Failure> {
        if !(1..=100).contains(&limit) {
            return Err(invalid("invalid_query"));
        }
        self.database.transaction(move |tx| Box::pin(async move {
            let rows = sqlx::query("SELECT event_id::text AS event_id,occurred_at_ms,recorded_at_ms,action,outcome,actor_kind,actor_identity,subject_id::text AS subject_id,tenant,work_id::text AS work_id,correlation_id::text AS correlation_id,causation_id::text AS causation_id,origin,details::text AS details FROM public.n2f_audit_records ORDER BY recorded_at_ms DESC,event_id DESC LIMIT $1")
                .bind(limit).fetch_all(&mut *tx).await.map_err(map)?;
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let details: Value = serde_json::from_str(row.try_get::<String,_>("details").map_err(map)?.as_str()).map_err(|_| invalid("invalid_record"))?;
                let event_id: String = row.try_get("event_id").map_err(map)?;
                let subject_id: String = row.try_get("subject_id").map_err(map)?;
                let work_id: String = row.try_get("work_id").map_err(map)?;
                let correlation_id: String = row.try_get("correlation_id").map_err(map)?;
                let causation_id: Option<String> = row.try_get("causation_id").map_err(map)?;
                out.push(serde_json::to_value(View {
                    id: event_id.clone(), event_id, occurred_at_ms: row.try_get("occurred_at_ms").map_err(map)?, recorded_at_ms: row.try_get("recorded_at_ms").map_err(map)?, action: row.try_get("action").map_err(map)?, outcome: row.try_get("outcome").map_err(map)?,
                    actor: ViewActor { kind: row.try_get("actor_kind").map_err(map)?, identity: row.try_get("actor_identity").map_err(map)? }, subject_id, tenant: row.try_get("tenant").map_err(map)?, work_id, correlation_id, causation_id, origin: row.try_get("origin").map_err(map)?, details,
                }).map_err(|_| invalid("invalid_record"))?);
            }
            Ok(out)
        })).await
    }
}

#[derive(Deserialize)]
struct EventWire {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    // The shared envelope deliberately writes time as a JSON number. Its
    // canonical encoding is often `123.0`, so decode as f64 and validate the
    // integral time range explicitly before narrowing it.
    occurred_at_ms: f64,
    payload: Value,
}
pub fn translate(event: &Envelope, recorded_at_ms: i64) -> Result<Option<Record>, Failure> {
    let wire: EventWire =
        serde_json::from_slice(event.bytes()).map_err(|_| invalid("invalid_event"))?;
    if Id::parse(&wire.id).map_err(|_| invalid("invalid_event"))? != event.id() {
        return Err(invalid("invalid_event"));
    }
    if !matches!(
        wire.event_type.as_str(),
        "identity.principal.registered.v1"
            | "identity.email.verified.v1"
            | "identity.session.created.v1"
            | "identity.session.revoked.v1"
            | "identity.sessions.revoked.v1"
            | "identity.password.changed.v1"
    ) {
        return Ok(None);
    }
    if !wire.occurred_at_ms.is_finite()
        || !(0.0..=253402300799999.0).contains(&wire.occurred_at_ms)
        || wire.occurred_at_ms.fract() != 0.0
    {
        return Err(invalid("invalid_event"));
    }
    let subject = wire
        .payload
        .get("principal_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("invalid_event"))?;
    let subject_id = Id::parse(subject).map_err(|_| invalid("invalid_event"))?;
    let work = event.work().snapshot();
    let attr = work.attribution.snapshot();
    let (kind, identity) = attr
        .initiator
        .map(|a| {
            (
                match a.kind() {
                    ActorKind::Anonymous => "anonymous",
                    ActorKind::User => "user",
                    ActorKind::Service => "service",
                    ActorKind::System => "system",
                }
                .into(),
                a.identity().into(),
            )
        })
        .unwrap_or_else(|| ("anonymous".into(), String::new()));
    Ok(Some(Record {
        id: event.id(),
        event_id: event.id(),
        occurred_at_ms: wire.occurred_at_ms as i64,
        recorded_at_ms,
        action: wire.event_type,
        outcome: "succeeded".into(),
        actor: Actor { kind, identity },
        subject_id,
        tenant: attr.tenant,
        work_id: work.work_id,
        correlation_id: work.correlation_id,
        causation_id: work.causation.map(|c| c.id()),
        origin: work.origin.map(|o| {
            match o {
                Origin::Request => "request",
                Origin::Schedule => "schedule",
                Origin::Replay => "replay",
                Origin::Backfill => "backfill",
                Origin::Startup => "startup",
            }
            .into()
        }),
        details: wire.payload,
    }))
}

fn query_limit(raw: &str) -> Result<i64, Failure> {
    let mut limit = 50;
    let mut seen = false;
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        if key == "limit" {
            if seen {
                return Err(invalid("invalid_query"));
            }
            seen = true;
            limit = value.parse().map_err(|_| invalid("invalid_query"))?;
        }
    }
    if !(1..=100).contains(&limit) {
        return Err(invalid("invalid_query"));
    }
    Ok(limit)
}
pub fn route(admission: Admit, store: Arc<Store>) -> Route {
    Route {
        path: "/v1/audit/records".into(),
        method: "GET".into(),
        operation: "audit.http.records".into(),
        admission: Some(admission),
        handler: Arc::new(move |context: RequestContext| {
            let store = store.clone();
            let limit = query_limit(&context.request.query);
            Box::pin(async move {
                let limit = limit.map_err(RequestFailure::Known)?;
                let records = store.list(limit).await.map_err(RequestFailure::Known)?;
                Ok(
                    Response::json(serde_json::json!({"records": records, "limit": limit}))
                        .with_header("Cache-Control", "no-store"),
                )
            })
        }),
    }
}
