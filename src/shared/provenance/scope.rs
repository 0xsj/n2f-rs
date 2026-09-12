use super::{Actor, ReferenceKind, WorkContext, WorkSnapshot, actor::invalid, restore_work};
use crate::shared::{errors::Failure, id::Id};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeSnapshot {
    pub work: WorkSnapshot,
    pub scope_id: Id,
    pub started_at: SystemTime,
    pub executor: Actor,
    pub attempt: u32,
    pub previous_attempt: Option<Id>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    work: WorkContext,
    scope_id: Id,
    started_at: SystemTime,
    executor: Actor,
    attempt: u32,
    previous_attempt: Option<Id>,
}
pub(super) fn normalized_time(t: SystemTime) -> Result<SystemTime, Failure> {
    let ms = t
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("invalid_time"))?
        .as_millis();
    if ms > 281474976710655 {
        return Err(invalid("invalid_time"));
    }
    Ok(UNIX_EPOCH + Duration::from_millis(ms as u64))
}
impl Scope {
    pub fn snapshot(&self) -> ScopeSnapshot {
        ScopeSnapshot {
            work: self.work.snapshot(),
            scope_id: self.scope_id,
            started_at: self.started_at,
            executor: self.executor.clone(),
            attempt: self.attempt,
            previous_attempt: self.previous_attempt,
        }
    }
    pub fn work_context(&self) -> &WorkContext {
        &self.work
    }
}
pub fn restore_scope(s: ScopeSnapshot) -> Result<Scope, Failure> {
    let work = restore_work(s.work)?;
    if !s.executor.named() {
        return Err(invalid("invalid_actor"));
    }
    if s.attempt == 0 {
        return Err(invalid("invalid_attempt"));
    }
    if s.previous_attempt
        .is_some_and(|v| v == s.scope_id || s.attempt == 1)
        || work
            .data
            .causation
            .is_some_and(|r| r.kind() == ReferenceKind::Scope && r.id() == s.scope_id)
    {
        return Err(invalid("invalid_scope"));
    }
    let started_at = normalized_time(s.started_at)?;
    Ok(Scope {
        work,
        scope_id: s.scope_id,
        started_at,
        executor: s.executor,
        attempt: s.attempt,
        previous_attempt: s.previous_attempt,
    })
}
