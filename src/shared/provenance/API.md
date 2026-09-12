# Rust provenance API

## Data shapes

These are implemented public values and operations. The signature summaries
use the names below; the [contract](CONTRACT.md) owns their semantics.
ScopeSnapshot contains a nested WorkSnapshot (`Work` in Go, `work` in Rust/TS);
it does not flatten or duplicate work fields. Native struct/property casing applies.

| Shape | Fields |
| --- | --- |
| Actor | kind; identity only for a named actor |
| Attribution | optional initiator, optional onBehalfOf, optional tenant |
| Reference | kind (`scope`, `work`, `event`), shared ID |
| ReplayInfo | runId, source Reference |
| WorkSnapshot | workId, correlationId, correlationSource, optional causation, optional origin, operation, Attribution, optional depth, optional ReplayInfo |
| ScopeSnapshot | WorkSnapshot, scopeId, startedAt, named executor, positive attempt, optional previousAttempt |
| RootSpec | optional workId, known non-replay origin, operation, Attribution, named executor |
| StepSpec | optional workId, operation, named executor, optional explicit cause |
| WorkSpec | required workId, operation, optional explicit cause |
| ExecutionSpec | named executor, positive owner-supplied attempt |
| ReplaySpec | source Reference, optional runId/workId, operation, Attribution, named executor; origin is selected by Replay |
| IncomingHints | optional correlation text; optional cause with kind text and ID text |
| IncomingResult | fresh/continued/restarted decision, accepted correlation/cause when applicable, ordered bounded issue values |
| Link | relation (`input`, `replay_of`, `previous_attempt`), Reference |
| LinkSet | immutable bounded collection; previous_attempt requires a scope Reference |

Input/snapshot records are not proof of valid construction. RestoreScope and
RestoreWork validate before creating values and do not authorize the claims.
Scope/WorkContext are constructed values with controlled read access. An exported
snapshot is an owned copy. No source actor, tenant, operation or replay data can
be changed through it.

Scope exposes execution fields through Snapshot/snapshot and a WorkContext
view/copy. WorkContext exposes work fields through its owned snapshot. Zero/missing
Scope and WorkContext are invalid. A default Attribution with all optional fields
absent means unknown initiator, no represented principal, no tenant; it does not
mean a known anonymous actor. Anonymous is constructed explicitly.

RestoreScope validates all work fields plus execution fields: positive attempt,
valid start time/executor, no self-cause/previous-attempt, and no previousAttempt
on attempt 1. An attempt greater than 1 may have no known previous attempt.
A replay origin and ReplayInfo must appear together. Known local depth 0 has no
primary cause; positive known depth requires one. Publicly adopted external
correlation has unknown origin/depth. A valid snapshot is still only a claim
until its adapter applies producer and application policy.

Prepare creates a child WorkContext without dependencies; Child is its immediate
execution convenience with attempt 1. Scope.workContext retains the same work;
it is not Prepare and does not add a hop. Retry uses that existing work context.

Root/Step/Replay work IDs and run IDs, when supplied, must be nonzero valid shared
IDs. Replay rejects a workId equal to a source work Reference's ID. A source
scope ID can be checked for generated execution collision; a bare Reference does
not expose an original correlation or a complete ancestry history.

## Native shape

Use ordinary owned structs and enums with private fields. The core does not
choose an async runtime, task-local storage or tracing crate. A factory can use
closures for clock and generation, adapting the existing ID generator through
`|| ids.new_id()` without importing its concrete implementation.

```rust,ignore
// Public signature summary; bodies are in the native source files.
// Factory<C, G> where C: Fn() -> SystemTime,
//                       G: FnMut() -> Result<Id, Failure>
Factory::new(clock, generate) -> Factory<C, G>
Factory::open(&mut self, spec: RootSpec) -> Result<Scope, Failure>
Factory::child(&mut self, parent: &Scope, spec: StepSpec) -> Result<Scope, Failure>
prepare(parent: &Scope, spec: WorkSpec) -> Result<WorkContext, Failure>
Factory::execute(&mut self, work: &WorkContext, spec: ExecutionSpec) -> Result<Scope, Failure>
Factory::retry(&mut self, previous: &Scope, executor: Actor) -> Result<Scope, Failure>
Factory::replay(&mut self, spec: ReplaySpec) -> Result<Scope, Failure>
inspect_incoming(hints: IncomingHints<'_>) -> IncomingResult
Factory::enter(&mut self, spec: RootSpec, incoming: &IncomingResult) -> Result<Scope, Failure>

Actor::new(kind: ActorKind, identity: String) -> Result<Actor, Failure>
Actor::anonymous() -> Actor
Attribution::new(spec: AttributionSpec) -> Result<Attribution, Failure>
Operation::new(name: String) -> Result<Operation, Failure>
Reference::new(kind: ReferenceKind, value: Id) -> Result<Reference, Failure>
LinkSet::new(links: Vec<Link>) -> Result<LinkSet, Failure>
restore_work(snapshot: WorkSnapshot) -> Result<WorkContext, Failure>
restore_scope(snapshot: ScopeSnapshot) -> Result<Scope, Failure>
Scope::snapshot(&self) -> ScopeSnapshot
Scope::work_context(&self) -> &WorkContext
WorkContext::snapshot(&self) -> WorkSnapshot
LinkSet::values(&self) -> &[Link]
```

The summary describes signatures, not a runnable Rust program. `Factory<C, G>`
uses `C: Fn() -> SystemTime` and `G: FnMut() -> Result<Id, Failure>`. `?` propagates a Failure unchanged instead of classifying it again.

Use Option for absent initiator/tenant/depth/cause/replay/previous attempt. The
anonymous Actor variant is a real value, distinct from None. Named actor variants
carry a validated identity. Avoid Default on Scope, WorkContext and Actor so a
missing value does not masquerade as a constructed one.

Actor/Operation/Attribution own strings; Clone is explicit where a child needs
an owned snapshot, and borrowed getters avoid incidental copies. Do not derive
Copy on values that own strings/collections. Id retains its existing Copy value
semantics. Readonly access to links can borrow a slice; snapshot/export paths clone
owned collections. No interior mutable field may let a Scope change afterward.

Attempt/depth use u32 with checked_add. Start time uses SystemTime, normalized to
milliseconds and checked against the common operational range before generation.
Factory methods use &mut self because generation is effectful. Caller-owned
synchronization handles shared factories; the core adds no async-runtime bound.
Restore functions validate owned snapshot data without touching clock or IDs;
serde/wire decoding, authentication and task propagation remain adapter work.
