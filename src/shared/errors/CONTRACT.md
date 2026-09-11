# Errors: first implementation contract

This slice turns the package documentation into executable behavior. The public
surface is small enough to test without accessing representation details. The
package/module documentation remains the context for ownership and future work.

## Comparable scenarios

| ID | Observable promise |
| --- | --- |
| E01 | The ten documented kinds have stable names; unknown names are rejected. |
| E02 | Construction preserves deliberately supplied classification and public data. |
| E03 | Success/absence is distinct from an existing unclassified failure. |
| E04 | Public projection hides the message, Type and fields of Internal and unknown failures. |
| E05 | Supplied metadata, derived values and public projections cannot mutate another error. |
| E06 | Adding diagnostic context preserves the local condition and its original cause. |
| E07 | Explicit translation selects the outer public data together and retains the diagnostic cause. |
| E08 | Classified cancellation and timeout survive annotation; arbitrary text does not classify errors. |
| E09 | Independent failures do not acquire the first branch's classification; an explicit summary can retain all outcomes. |
| E10 | Metadata merges keep unrelated keys, new values replace collisions, and source replacement preserves the complete meaning and diagnostic frame. |

The unknown-plus-transient case belongs to E09. There is no retry predicate in this
slice. Enumeration order is not a compatibility promise; membership and names are.
Recovery, idempotency and commit uncertainty remain operation concerns.
Logging discipline and dependency direction are review rules, not behavior claims
proved by this suite.

Inputs and outputs are tested through the public surface. Metadata means maps of
strings to strings. Empty Type means no identifier. Missing public messages on
non-Internal failures become `request failed`; Internal/unknown become
`internal error`. Diagnostic details remain available to internal callers.

Adding a source deliberately creates a new occurrence with that source, replacing
any source on the template. It preserves the condition and metadata. It does not
append unrelated causes into an aggregate. Annotation preserves the current
source instead. Foreign causes are opaque references/owned values; this package
does not make arbitrary external objects deeply immutable.

Each implementation exposes a complete public view (kind, message, optional Type
and fields) selected from one classification frame. Empty Type is absent even
after annotation. This is a transport-neutral value; JSON names, status codes and
HTTP/WebSocket envelopes belong to later adapters.

## Test procedure

Tests are written against this contract and the declared API before method bodies
are completed. Compile-ready placeholders allow an initial run to demonstrate
behavioral assertion failures. Record that run, then implement and refactor.

This is ordinary specification-first TDD with implementation visibility available
to the author. It is not an independently authored or implementation-blind suite.
Targeted mutation checks supplement the green run; they measure only the selected
faults and do not establish completeness.

## Rust surface

`Kind` supplies `ALL`, `as_str` and `parse`.
`Classification<'a>` is a borrowed view of kind, error_type, message and fields.
Domain-owned error types implement `Classified: std::error::Error`, whose
`classification()` defaults to None. No domain registry or universal downcast
walker is involved.

`Failure` supplies owned shared error data and consuming `with_type`,
`with_field(s)`, `with_detail(s)` and `with_source` builders.
`details()` borrows immutable metadata. Its boxed source is Error + Send + Sync +
'static so the shared carrier can cross worker boundaries; Classified itself
does not impose these bounds on domain errors. Sources need not implement Clone.

`Context<E>` holds a typed error, operation text, `inner()` and `into_inner()`.
It delegates Classified, exposes E through source(), and does not erase its case.
`public_info(Option<Classification>) -> PublicInfo` creates an independent public
view for an actual failure. None here means an unclassified failure, not a
successful Result. Call it only on the Err branch. Ok and optional absence retain
their ordinary standard-library meanings.

An aggregate is owned by its operation and explicitly supplies no classification
unless it defines a summary. The example fixture retains all its items. This
slice does not provide or claim a general aggregate/source-tree inspector.

## Rust boxed-source extension

E11: `Failure::with_boxed_source` accepts an already owned
`Box<dyn Error + Send + Sync + 'static>`. It replaces the existing source without
adding another wrapper, so `source().downcast_ref` reaches the original concrete
error directly. Classification, public fields and private details remain intact.
The ID entropy boundary is the first consumer of this extension.
