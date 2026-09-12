//! Durable event envelopes and the dispatcher's replaceable publisher contract.
pub mod jetstream;
pub mod postgres;
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
    provenance as p,
};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
#[derive(Clone)]
pub struct Envelope {
    id: Id,
    raw: Vec<u8>,
    work: p::WorkContext,
}
impl Envelope {
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
    pub fn work(&self) -> &p::WorkContext {
        &self.work
    }
    pub fn new(
        id: Id,
        name: &str,
        occurred_at: i64,
        work: &p::WorkContext,
        payload: serde_json::Value,
    ) -> Result<Self, Failure> {
        let w = Wire {
            v: 1.0,
            id: id.to_string(),
            event_type: name.into(),
            occurred_at_ms: occurred_at as f64,
            work: work_out(work),
            payload,
        };
        Self::decode(&serde_json::to_vec(&w).map_err(|_| invalid())?)
    }
    pub fn decode(raw: &[u8]) -> Result<Self, Failure> {
        if raw.len() > 65536 {
            return Err(invalid());
        }
        let mut w: Wire = serde_json::from_slice(raw).map_err(|_| invalid())?;
        if w.v != 1.0
            || !valid_type(&w.event_type)
            || !(0.0..=253402300799999.0).contains(&w.occurred_at_ms)
            || w.occurred_at_ms.fract() != 0.0
            || !w.payload.is_object()
        {
            return Err(invalid());
        }
        let id = Id::parse(&w.id)?;
        let work = work_in(w.work)?;
        w.work = work_out(&work);
        w.id = id.to_string();
        let raw = serde_json::to_vec(&w).map_err(|_| invalid())?;
        if raw.len() > 65536 {
            return Err(invalid());
        }
        Ok(Self { id, raw, work })
    }
}
pub struct Receipt {
    pub event_id: Id,
    pub durable: bool,
}
/// An implementation owns a finite I/O budget; caller cancellation drops its future.
pub trait Publisher: Send + Sync {
    fn publish<'a>(&'a self, event: &'a Envelope) -> BoxFuture<'a, Result<Receipt, Failure>>;
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid event envelope").with_type("events.invalid")
}
fn valid_type(s: &str) -> bool {
    let Some((prefix, version)) = s.rsplit_once(".v") else {
        return false;
    };
    s.len() <= 128
        && !prefix.is_empty()
        && prefix.len() <= 120
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        && !version.is_empty()
        && version.len() <= 6
        && !version.starts_with('0')
        && version.bytes().all(|b| b.is_ascii_digit())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    v: f64,
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    occurred_at_ms: f64,
    work: WorkWire,
    payload: serde_json::Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActorWire {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RefWire {
    kind: String,
    id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttrWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    initiator: Option<ActorWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    on_behalf_of: Option<ActorWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayWire {
    run_id: String,
    source: RefWire,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkWire {
    work_id: String,
    correlation_id: String,
    correlation_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    causation: Option<RefWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    operation: String,
    attribution: AttrWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replay: Option<ReplayWire>,
}
fn actor_out(a: p::Actor) -> ActorWire {
    ActorWire {
        kind: match a.kind() {
            p::ActorKind::Anonymous => "anonymous",
            p::ActorKind::User => "user",
            p::ActorKind::Service => "service",
            p::ActorKind::System => "system",
        }
        .into(),
        identity: if a.identity().is_empty() {
            None
        } else {
            Some(a.identity().into())
        },
    }
}
fn actor_in(a: ActorWire) -> Result<p::Actor, Failure> {
    let kind = match a.kind.as_str() {
        "anonymous" => p::ActorKind::Anonymous,
        "user" => p::ActorKind::User,
        "service" => p::ActorKind::Service,
        "system" => p::ActorKind::System,
        _ => return Err(invalid()),
    };
    p::Actor::new(kind, a.identity.unwrap_or_default())
}
fn ref_out(r: p::Reference) -> RefWire {
    RefWire {
        kind: match r.kind() {
            p::ReferenceKind::Scope => "scope",
            p::ReferenceKind::Work => "work",
            p::ReferenceKind::Event => "event",
        }
        .into(),
        id: r.id().to_string(),
    }
}
fn ref_in(r: RefWire) -> Result<p::Reference, Failure> {
    p::Reference::new(
        match r.kind.as_str() {
            "scope" => p::ReferenceKind::Scope,
            "work" => p::ReferenceKind::Work,
            "event" => p::ReferenceKind::Event,
            _ => return Err(invalid()),
        },
        Id::parse(&r.id)?,
    )
}
fn work_out(work: &p::WorkContext) -> WorkWire {
    let w = work.snapshot();
    let a = w.attribution.snapshot();
    WorkWire {
        work_id: w.work_id.to_string(),
        correlation_id: w.correlation_id.to_string(),
        correlation_source: match w.correlation_source {
            p::CorrelationSource::Local => "local",
            p::CorrelationSource::External => "external",
        }
        .into(),
        causation: w.causation.map(ref_out),
        origin: w.origin.map(|o| {
            match o {
                p::Origin::Request => "request",
                p::Origin::Schedule => "schedule",
                p::Origin::Replay => "replay",
                p::Origin::Backfill => "backfill",
                p::Origin::Startup => "startup",
            }
            .into()
        }),
        operation: w.operation.as_str().into(),
        attribution: AttrWire {
            initiator: a.initiator.map(actor_out),
            on_behalf_of: a.on_behalf_of.map(actor_out),
            tenant: a.tenant,
        },
        depth: w.depth,
        replay: w.replay.map(|r| ReplayWire {
            run_id: r.run_id.to_string(),
            source: ref_out(r.source),
        }),
    }
}
fn work_in(w: WorkWire) -> Result<p::WorkContext, Failure> {
    p::restore_work(p::WorkSnapshot {
        work_id: Id::parse(&w.work_id)?,
        correlation_id: Id::parse(&w.correlation_id)?,
        correlation_source: match w.correlation_source.as_str() {
            "local" => p::CorrelationSource::Local,
            "external" => p::CorrelationSource::External,
            _ => return Err(invalid()),
        },
        causation: w.causation.map(ref_in).transpose()?,
        origin: w
            .origin
            .map(|s| match s.as_str() {
                "request" => Ok(p::Origin::Request),
                "schedule" => Ok(p::Origin::Schedule),
                "replay" => Ok(p::Origin::Replay),
                "backfill" => Ok(p::Origin::Backfill),
                "startup" => Ok(p::Origin::Startup),
                _ => Err(invalid()),
            })
            .transpose()?,
        operation: p::Operation::new(w.operation)?,
        attribution: p::Attribution::new(p::AttributionSpec {
            initiator: w.attribution.initiator.map(actor_in).transpose()?,
            on_behalf_of: w.attribution.on_behalf_of.map(actor_in).transpose()?,
            tenant: w.attribution.tenant,
        })?,
        depth: w.depth,
        replay: w
            .replay
            .map(|r| {
                Ok(p::ReplayInfo {
                    run_id: Id::parse(&r.run_id)?,
                    source: ref_in(r.source)?,
                })
            })
            .transpose()?,
    })
}
