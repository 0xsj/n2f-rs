use crate::shared::{
    errors::{Classification, public_info},
    provenance as p,
};
use serde_json::{Value, json};
use std::{error::Error, time::UNIX_EPOCH};
pub(super) fn error(error: &dyn Error, classification: Option<Classification<'_>>) -> Value {
    let public = public_info(classification);
    let mut p = json!({"kind":public.kind.as_str(),"message":public.message});
    if let Some(t) = public.error_type {
        p["type"] = t.into();
    }
    if !public.fields.is_empty() {
        p["fields"] = json!(public.fields);
    }
    let mut v = json!({"classified":classification.is_some(),"kind":classification.map_or("internal",|c|c.kind.as_str()),"public":p,"has_cause":error.source().is_some()});
    if let Some(t) = classification.and_then(|c| c.error_type) {
        v["type"] = t.into();
    }
    v
}
fn actor(a: &p::Actor) -> Value {
    let kind = match a.kind() {
        p::ActorKind::Anonymous => "anonymous",
        p::ActorKind::User => "user",
        p::ActorKind::Service => "service",
        p::ActorKind::System => "system",
    };
    let mut v = json!({"kind":kind});
    if !a.identity().is_empty() {
        v["identity"] = a.identity().into();
    }
    v
}
fn reference(r: p::Reference) -> Value {
    json!({"kind":match r.kind(){p::ReferenceKind::Scope=>"scope",p::ReferenceKind::Work=>"work",p::ReferenceKind::Event=>"event"},"id":r.id().to_string()})
}
pub(super) fn scope(scope: &p::Scope) -> Value {
    let s = scope.snapshot();
    let w = s.work;
    let a = w.attribution.snapshot();
    let mut at = json!({});
    if let Some(a) = a.initiator {
        at["initiator"] = actor(&a);
    }
    if let Some(a) = a.on_behalf_of {
        at["on_behalf_of"] = actor(&a);
    }
    if let Some(t) = a.tenant {
        at["tenant"] = t.into();
    }
    let mut v = json!({"scope_id":s.scope_id.to_string(),"work_id":w.work_id.to_string(),"correlation_id":w.correlation_id.to_string(),"correlation_source":match w.correlation_source{p::CorrelationSource::Local=>"local",p::CorrelationSource::External=>"external"},"operation":w.operation.as_str(),"started_at_ms":s.started_at.duration_since(UNIX_EPOCH).expect("validated scope time").as_millis() as u64,"executor":actor(&s.executor),"attempt":s.attempt,"attribution":at});
    if let Some(o) = w.origin {
        v["origin"] = match o {
            p::Origin::Request => "request",
            p::Origin::Schedule => "schedule",
            p::Origin::Replay => "replay",
            p::Origin::Backfill => "backfill",
            p::Origin::Startup => "startup",
        }
        .into();
    }
    if let Some(d) = w.depth {
        v["depth"] = d.into();
    }
    if let Some(c) = w.causation {
        v["causation"] = reference(c);
    }
    if let Some(prev) = s.previous_attempt {
        v["previous_attempt"] = prev.to_string().into();
    }
    if let Some(r) = w.replay {
        v["replay"] = json!({"run_id":r.run_id.to_string(),"source":reference(r.source)});
    }
    v
}
