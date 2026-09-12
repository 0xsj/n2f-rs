//! Provenance describes one execution, its logical work and the history it knows.
//!
//! The contract includes all accepted additions. Scope is an immutable execution
//! value; WorkContext preserves operation and attribution across attempts without
//! claiming that prepared work has started. Each execution has a fresh scope ID and
//! an explicit wall-clock start time. A retry keeps work/correlation/cause/depth,
//! opens a new scope, increments the owner-visible ordinal and names a known prior
//! attempt. A deliberate replay starts a new chain with current attribution and
//! retains its original source and replay-run identity.
//!
//! Initiator, executor and represented principal have different lifetimes. Tenant
//! and initiator are inherited by child work; executor and operation are explicit
//! at each new step. Unknown initiator is distinct from known anonymous work.
//! Delegation is validated as a combination; no independent setter can invalidate
//! it after construction. These are recorded claims, never authorization grants.
//!
//! A primary cause expresses the immediate trigger. Bounded typed LinkSet values
//! on an owning work/event/audit envelope retain additional input, replay and prior
//! attempt relationships without copying complete history into every Scope.
//! Preparing a WorkContext, holding a reference or recording depth proves no commit,
//! publication, target existence, cycle detection or durable audit write.
//!
//! Incoming public correlation hints are inspected separately from stored or
//! producer-authenticated work. Fresh, continued and restarted boundaries remain
//! distinguishable. Unusable optional hints can be discarded without refusing
//! otherwise valid work; malformed stored work never silently becomes a fresh root.
//! Unknown upstream origin/depth remains unknown. Scope IDs, actors, tenants,
//! attempts, start times and replay claims are never adopted from public hints.
//!
//! WebSocket connections and message executions have separate lifetimes. Runtime
//! context storage, HTTP/broker codecs, log projections and trace propagation are
//! adapters, not dependencies of the value model. Event occurrence and record time
//! remain distinct from the scope's execution start time.
//!
//! Stage: implemented core. CONTRACT.md defines P01-P24 and exact transitions;
//! API.md describes native interfaces; SCENARIOS.md contains worked cases.
//! Core scenarios are executable; transport and durability adapters remain separate.
//!
//! The complete local acceptance contract follows.
#![doc = include_str!("CONTRACT.md")]
mod actor;
mod attribution;
mod factory;
mod incoming;
mod links;
mod operation;
mod reference;
mod scope;
mod work;
pub use actor::{Actor, ActorKind};
pub use attribution::{Attribution, AttributionSpec};
pub use factory::{ExecutionSpec, Factory, ReplaySpec, RootSpec, StepSpec, WorkSpec, prepare};
pub use incoming::{
    Disposition, IncomingCause, IncomingHints, IncomingResult, Issue, inspect_incoming,
};
pub use links::{Link, LinkSet, Relation};
pub use operation::Operation;
pub use reference::{Reference, ReferenceKind};
pub use scope::{Scope, ScopeSnapshot, restore_scope};
pub use work::{CorrelationSource, Origin, ReplayInfo, WorkContext, WorkSnapshot, restore_work};
