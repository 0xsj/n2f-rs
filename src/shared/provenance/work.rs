use super::{Attribution, Operation, Reference, ReferenceKind, actor::invalid};
use crate::shared::{errors::Failure, id::Id};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Request,
    Schedule,
    Replay,
    Backfill,
    Startup,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrelationSource {
    Local,
    External,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayInfo {
    pub run_id: Id,
    pub source: Reference,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkSnapshot {
    pub work_id: Id,
    pub correlation_id: Id,
    pub correlation_source: CorrelationSource,
    pub causation: Option<Reference>,
    pub origin: Option<Origin>,
    pub operation: Operation,
    pub attribution: Attribution,
    pub depth: Option<u32>,
    pub replay: Option<ReplayInfo>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkContext {
    pub(super) data: WorkSnapshot,
}
impl WorkContext {
    pub fn snapshot(&self) -> WorkSnapshot {
        self.data.clone()
    }
}
pub(super) fn validate_work(s: &WorkSnapshot, known_work: bool) -> Result<(), Failure> {
    if s.replay.is_some() != (s.origin == Some(Origin::Replay)) {
        return Err(invalid("invalid_origin"));
    }
    if known_work
        && s.causation
            .is_some_and(|r| r.kind() == ReferenceKind::Work && r.id() == s.work_id)
    {
        return Err(invalid("invalid_work"));
    }
    if let Some(d) = s.depth {
        if (s.correlation_source == CorrelationSource::Local && d == 0 && s.causation.is_some())
            || (d > 0 && s.causation.is_none())
        {
            return Err(invalid("invalid_depth"));
        }
    }
    if known_work
        && s.replay
            .as_ref()
            .is_some_and(|r| r.source.kind() == ReferenceKind::Work && r.source.id() == s.work_id)
    {
        return Err(invalid("invalid_work"));
    }
    Ok(())
}
pub fn restore_work(data: WorkSnapshot) -> Result<WorkContext, Failure> {
    validate_work(&data, true)?;
    Ok(WorkContext { data })
}
