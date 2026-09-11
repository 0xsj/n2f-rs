//! The foundation for describing failures across domain, application,
//! infrastructure and transport boundaries.
//!
//! The first implementation covers classification, safe public projection,
//! owned metadata and typed context/source preservation. CONTRACT.md records the
//! concrete choices and scenarios E01-E10, exercised in tests/errors_spec.rs.
//! The complete contract lives here without a sibling repository dependency.
//!
//! # Why this module comes first
//!
//! A domain must explain a refusal without knowing its database, logger or
//! transport. An adapter must translate a dependency failure without requiring
//! callers to understand that dependency. A small shared vocabulary supports both.
//!
//! This is a leaf module: standard-library dependencies only, no imports from
//! other project modules, no I/O, configuration, registration or background work.
//! Logging, provenance and transport may depend on errors; errors must not depend
//! on them. Request IDs and trace IDs belong to the observing boundary. Runtime
//! cancellation tokens, active spans and framework request objects stay outside.
//!
//! # Three different questions
//!
//! `Kind` answers "what category of failure is this?" It is a small closed
//! vocabulary shared across modules and backend blueprints.
//!
//! `Type` answers "which particular condition occurred?" It is an optional,
//! stable machine-readable string owned by the module defining the condition,
//! such as `account.email_taken`. Type is the conceptual name shared with the
//! frontends; a Rust accessor can be named `error_type`. It does not identify a
//! Rust concrete type. Changing a published Type can break consumers. Messages
//! are prose and must never be used as identifiers.
//!
//! A domain enum variant or concrete error type answers "which local case can
//! this caller handle?" Callers match typed cases. A shared Kind does not replace
//! them, and this module does not own every domain's variants. Rust does not need
//! Go's sentinel pointer identity: adding context must preserve the recoverable
//! local case and its data, not an allocation address or message equality.
//!
//! # Initial kinds
//!
//! Start with these meanings. Wire status codes belong to transports.
//!
//! - `Invalid`: input fails validation; field problems may explain corrections.
//! - `NotFound`: a required resource does not exist or is not visible to this caller.
//! - `Conflict`: the operation conflicts with current state or a uniqueness rule.
//! - `Unauthenticated`: acceptable authentication is absent or invalid.
//! - `Forbidden`: the caller is not permitted to perform the operation.
//! - `RateLimited`: an applicable rate or usage limit currently refuses the request.
//! - `Unavailable`: a capability needed by the operation is currently unavailable.
//! - `Timeout`: the operation's allowed time elapsed; completion may be uncertain.
//! - `Canceled`: the operation was canceled; completion may be uncertain.
//! - `Internal`: an explicitly classified unexpected failure.
//!
//! Textual forms are lower snake case, including `not_found` and `rate_limited`.
//! Transport numbers are not enum discriminants. Add a Kind only for a distinct
//! shared meaning with a concrete caller; use a domain Type or variant for a more
//! specific rule. Preconditions and richer validation categories can follow a
//! real contract that needs them.
//!
//! A foreign error remains unclassified until an owning boundary recognizes it.
//! Presentation falls back to Internal without pretending classification was
//! supplied. Unlike Go's context errors, this contract has no universal runtime
//! cancellation or deadline source to recognize. An adapter maps a known timeout
//! or cancellation into the shared vocabulary; annotation preserves that meaning.
//! An unrelated cancellation signal does not classify a returned failure.
//!
//! # Public information and private diagnostics
//!
//! The shared Failure representation carries Kind, optional Type, a
//! public message, public field problems, private diagnostic details and an
//! optional source. Fields map input names to safe explanations. Details provide
//! operator context and are not additional response fields.
//!
//! `Display`, `Debug` and `std::error::Error::source` serve diagnostics. Do not use
//! `to_string()` or a debug rendering as the public message, or serialize the
//! error and its source chain directly. A transport projects the public vocabulary
//! explicitly and supplies its own correlation information.
//!
//! The public projection is `internal error`, with no Type or Fields, for Internal
//! and unclassified failures. A classified non-Internal error exposes only its
//! owner's explicitly public message, Type and Fields; an empty message falls
//! back to `request failed`. An absent error has no public failure projection.
//!
//! Never fill missing public metadata from a private source. Messages and field
//! problems must not contain secrets or implementation details. Private data can
//! still be unsafe to log: avoid collecting secrets and let the observing
//! boundary apply its redaction policy. Diagnostics never determine public data.
//!
//! # Construct, annotate, translate
//!
//! Prefer `Result<T, E>` with domain- or operation-owned error enums where callers
//! need to distinguish cases. The shared representation is available where a
//! common structured failure is useful; it is not a mandatory replacement for E.
//! A module exposes its classification through an explicit mapping or a small
//! shared trait. This leaf must not import domains to downcast every known enum.
//!
//! Annotation adds private operation context while retaining classification,
//! public data, the typed local case and its source. Use `map_err` for deliberate
//! context or translation. Propagation with `?` must not introduce a broad From
//! conversion that silently turns distinct cases into Internal.
//!
//! Translation deliberately changes public meaning at an owned boundary. An
//! adapter can recognize a uniqueness violation as an email-taken domain variant
//! while retaining the original failure as a diagnostic source. Domain contracts
//! must not expose a driver type as a required field type. A type-erased source
//! can retain diagnostics without forcing callers to import that driver.
//!
//! The outermost explicit classification wins. Its Kind, Type, message and Fields
//! are selected together; missing public metadata is not filled from inner sources.
//! An annotation wrapper must explicitly delegate classification to its inner
//! error. After translation, inspecting an inner source and selecting the current
//! public classification answer different questions.
//!
//! Implement `std::error::Error` for error representations that participate in
//! diagnostic chains. Its `source()` exposes a cause, not a Kind or a public
//! projection; arbitrary `dyn Error` values cannot supply our classification
//! contract automatically. Rendering a source once belongs to the diagnostic
//! formatter, rather than duplicating a full source chain at every layer.
//!
//! # Ownership and Rust surface
//!
//! Keep owned metadata private after construction. Consuming builders may move
//! their input into a new value; borrowing helpers must preserve the original.
//! Borrowed inspection must not grant mutation, and owned projections must be
//! independent. Replacing a metadata key affects only the new value, with the
//! new value winning. Sharing a source must not expose mutable shared metadata.
//!
//! A source need not implement Clone or Eq. Do not require these bounds merely
//! to imitate Go's fluent cloning or sentinel comparisons. Context retains a
//! typed E. Failure owns a boxed Error + Send + Sync + 'static source, so this
//! shared carrier can cross worker boundaries. Classified itself does not impose
//! those source bounds on domain error types.
//!
//! Implemented roles:
//!
//! ```text
//! Kind                         shared category enum
//! Failure                      structured shared failure where needed
//! Classified::classification   inspect an explicit borrowed Classification
//! Classification::error_type   stable domain-owned condition identifier
//! public_info                  read the safe message, Type and Fields together
//! Context<E> / with_detail(s)   enrich diagnostics without changing local meaning
//! Failure::with_source         deliberately attach a replacement source
//! Failure::with_boxed_source   accept an already boxed source without reboxing
//! Result<T, E>                 standard Result, keeping the caller's error type
//! ```
//!
//! Classified explicitly returns a borrowed frame; arbitrary foreign sources do
//! not acquire classification automatically. public_info(None) projects an actual
//! unclassified failure and is called only on an Err path. Context delegates the
//! frame while retaining its typed inner value. No derive, reporting,
//! serialization or async-runtime dependency is needed for this implementation.
//!
//! # Success, absence and unknown failures
//!
//! `Ok` means success and `Err` means failure. `Result<Option<T>, E>` can express
//! a successful optional lookup separately from a failed read; an operation that
//! requires the resource translates absence into its NotFound case.
//!
//! Annotation through `map_err` leaves Ok untouched. Classification uses
//! `Option<Kind>`: None is not Internal, and Some(Internal) represents an explicit
//! classification. An optional error being None is absence; an existing error
//! with no recognized Kind is an unclassified failure. Do not confuse the two
//! or default successful outcomes into Internal assertions.
//!
//! Constructors create real failures. No nil-style wrapping API is needed, and
//! panics are outside the expected-failure vocabulary. Do not turn every panic
//! into a routine domain refusal or promise recovery where the process cannot
//! safely continue.
//!
//! # Multiple failures and partial outcomes
//!
//! A source chain describes one causal path. Several independent failures need
//! explicit item outcomes; `source()` is not a batch enumeration interface. An
//! aggregate does not acquire a Kind by selecting the first item or source.
//! Single-failure inspection treats an aggregate without an explicit summary
//! classification as unclassified. A deliberate outer summary may retain all
//! underlying outcomes without exposing them as one merged public error.
//!
//! Do not merge unrelated public fields or diagnostic details into one map. The
//! initial module offers no batch classifier, flattened diagnostic view or
//! aggregate retry predicate. A future contract must retain item identity,
//! unknown failures and partial successes, and define ordering. Reordering items
//! must not change the public summary. Unknown failures are not harmless branches.
//!
//! # Ownership of recovery and observation
//!
//! Kinds describe failures; they do not authorize retries. RateLimited,
//! Unavailable and Timeout may inform policy, but the operation owner decides
//! whether repetition is safe, whether a write already committed, and what budget
//! remains. Cancellation normally stops the canceled operation. Backoff,
//! idempotency, retry schedules and dead-letter decisions are outside this module.
//! Timeout or cancellation is not evidence of rollback.
//!
//! Lower layers return errors with context. The boundary handling the failure
//! owns observation; propagating layers do not each log it. This module neither
//! logs nor assigns log levels, HTTP status, WebSocket close codes, tracing status
//! or metric labels. Those mappings, including expected refusals, need their own
//! contracts. The leaf does not select or initialize an observability library.
//!
//! Empty results and valid domain outcomes remain successes. A missing required
//! record, refused transition, failed read and uncertain write stay distinct when
//! they require different caller behavior. Not every interesting state is an error.
//!
//! # Scenarios for the first implementation
//!
//! - Ok and absent optional errors remain successful/absent through annotation.
//! - An unclassified foreign failure exposes no private information to a caller.
//! - Adapter-classified cancellation and deadlines survive context annotation.
//! - Annotation retains the typed domain case, public classification and source.
//! - Translation changes public meaning while retaining the original diagnostic cause.
//! - Enriching one failure cannot change another use of its metadata.
//! - Borrowed or owned projections cannot mutate the originating failure.
//! - Internal failures conceal their message, Type and Fields in public accessors.
//! - Reordering aggregate items does not select a different public failure.
//! - An unknown failure alongside a transient one does not imply safe retry.
//!
//! These mirror the other blueprints' scenarios through Rust semantics. Common
//! meaning, disclosure and ownership do not require matching Go methods or
//! TypeScript representations. HTTP problem envelopes, WebSocket messages and
//! client mappings are later adapter contracts. This module does not establish
//! frontend integration. Its executable guarantees are listed in CONTRACT.md.

mod classification;
mod context;
mod failure;
mod kind;
mod public;

pub use classification::{Classification, Classified};
pub use context::Context;
pub use failure::Failure;
pub use kind::Kind;
pub use public::{PublicInfo, public_info};
