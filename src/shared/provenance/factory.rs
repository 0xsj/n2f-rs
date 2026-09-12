use super::{
    Actor, Attribution, CorrelationSource, Disposition, IncomingResult, Operation, Origin,
    Reference, ReferenceKind, ReplayInfo, Scope, ScopeSnapshot, WorkContext, WorkSnapshot,
    actor::invalid, restore_scope, restore_work, scope::normalized_time, work::validate_work,
};
use crate::shared::{
    errors::{Failure, Kind},
    id::Id,
};
use std::time::SystemTime;
pub struct RootSpec {
    pub work_id: Option<Id>,
    pub origin: Origin,
    pub operation: Operation,
    pub attribution: Attribution,
    pub executor: Actor,
}
pub struct StepSpec {
    pub work_id: Option<Id>,
    pub operation: Operation,
    pub executor: Actor,
    pub cause: Option<Reference>,
}
pub struct WorkSpec {
    pub work_id: Id,
    pub operation: Operation,
    pub cause: Option<Reference>,
}
pub struct ExecutionSpec {
    pub executor: Actor,
    pub attempt: u32,
}
pub struct ReplaySpec {
    pub source: Reference,
    pub run_id: Option<Id>,
    pub work_id: Option<Id>,
    pub operation: Operation,
    pub attribution: Attribution,
    pub executor: Actor,
}
pub struct Factory<C, G> {
    clock: C,
    generate: G,
}
fn generated_error() -> Failure {
    Failure::new(Kind::Internal, "invalid generated provenance ID")
        .with_type("provenance.invalid_generated_id")
}
fn child_work(
    parent: &Scope,
    work_id: Option<Id>,
    operation: Operation,
    cause: Option<Reference>,
) -> Result<WorkSnapshot, Failure> {
    let parent = parent.snapshot();
    let mut w = parent.work;
    if work_id == Some(w.work_id) {
        return Err(invalid("invalid_work"));
    }
    w.work_id = work_id.unwrap_or(w.work_id);
    w.operation = operation;
    w.depth = w
        .depth
        .map(|d| d.checked_add(1).ok_or_else(|| invalid("depth_exhausted")))
        .transpose()?;
    w.causation = Some(cause.unwrap_or(Reference::new(ReferenceKind::Scope, parent.scope_id)?));
    validate_work(&w, work_id.is_some())?;
    Ok(w)
}
pub fn prepare(parent: &Scope, s: WorkSpec) -> Result<WorkContext, Failure> {
    restore_work(child_work(parent, Some(s.work_id), s.operation, s.cause)?)
}
impl<C: Fn() -> SystemTime, G: FnMut() -> Result<Id, Failure>> Factory<C, G> {
    pub fn new(clock: C, generate: G) -> Self {
        Self { clock, generate }
    }
    fn emit(
        &mut self,
        executor: Actor,
        attempt: u32,
        previous: Option<Id>,
        forbidden: &[Id],
        build: impl FnOnce(Id) -> WorkSnapshot,
    ) -> Result<Scope, Failure> {
        if !executor.named() {
            return Err(invalid("invalid_actor"));
        }
        if attempt == 0 {
            return Err(invalid("invalid_attempt"));
        }
        let started_at = normalized_time((self.clock)())?;
        let scope_id = (self.generate)()?;
        if forbidden.contains(&scope_id) {
            return Err(generated_error());
        }
        restore_scope(ScopeSnapshot {
            work: build(scope_id),
            scope_id,
            started_at,
            executor,
            attempt,
            previous_attempt: previous,
        })
        .map_err(|_| generated_error())
    }
    pub fn open(&mut self, s: RootSpec) -> Result<Scope, Failure> {
        if s.origin == Origin::Replay {
            return Err(invalid("invalid_origin"));
        }
        self.emit(s.executor, 1, None, &[], |id| WorkSnapshot {
            work_id: s.work_id.unwrap_or(id),
            correlation_id: id,
            correlation_source: CorrelationSource::Local,
            causation: None,
            origin: Some(s.origin),
            operation: s.operation,
            attribution: s.attribution,
            depth: Some(0),
            replay: None,
        })
    }
    pub fn child(&mut self, parent: &Scope, s: StepSpec) -> Result<Scope, Failure> {
        let mut work = child_work(parent, s.work_id, s.operation, s.cause)?;
        let mut forbidden = vec![parent.snapshot().scope_id];
        if s.work_id.is_none() {
            forbidden.push(parent.work_context().data.work_id)
        }
        self.emit(s.executor, 1, None, &forbidden, |id| {
            if s.work_id.is_none() {
                work.work_id = id
            }
            work
        })
    }
    pub fn execute(&mut self, work: &WorkContext, s: ExecutionSpec) -> Result<Scope, Failure> {
        self.emit(s.executor, s.attempt, None, &[], |_| work.snapshot())
    }
    pub fn retry(&mut self, previous: &Scope, executor: Actor) -> Result<Scope, Failure> {
        let s = previous.snapshot();
        let attempt = s
            .attempt
            .checked_add(1)
            .ok_or_else(|| invalid("attempt_exhausted"))?;
        self.emit(executor, attempt, Some(s.scope_id), &[s.scope_id], |_| {
            s.work
        })
    }
    pub fn replay(&mut self, s: ReplaySpec) -> Result<Scope, Failure> {
        if s.source.kind() == ReferenceKind::Work && s.work_id == Some(s.source.id()) {
            return Err(invalid("invalid_work"));
        }
        let forbidden = if s.source.kind() == ReferenceKind::Scope {
            vec![s.source.id()]
        } else {
            vec![]
        };
        self.emit(s.executor, 1, None, &forbidden, |id| WorkSnapshot {
            work_id: s.work_id.unwrap_or(id),
            correlation_id: id,
            correlation_source: CorrelationSource::Local,
            causation: None,
            origin: Some(Origin::Replay),
            operation: s.operation,
            attribution: s.attribution,
            depth: Some(0),
            replay: Some(ReplayInfo {
                run_id: s.run_id.unwrap_or(id),
                source: s.source,
            }),
        })
    }
    pub fn enter(&mut self, s: RootSpec, incoming: &IncomingResult) -> Result<Scope, Failure> {
        if incoming.decision != Disposition::Continued {
            return self.open(s);
        }
        if s.origin == Origin::Replay {
            return Err(invalid("invalid_origin"));
        }
        if incoming
            .cause
            .is_some_and(|c| c.kind() == ReferenceKind::Work && Some(c.id()) == s.work_id)
        {
            return Err(invalid("invalid_work"));
        }
        self.emit(s.executor, 1, None, &[], |id| WorkSnapshot {
            work_id: s.work_id.unwrap_or(id),
            correlation_id: incoming
                .correlation
                .expect("constructed incoming correlation"),
            correlation_source: CorrelationSource::External,
            causation: incoming.cause,
            origin: None,
            operation: s.operation,
            attribution: s.attribution,
            depth: None,
            replay: None,
        })
    }
}
