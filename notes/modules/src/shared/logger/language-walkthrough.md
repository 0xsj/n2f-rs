# Logger: implementation walkthrough

The log envelope owns service, scope and error. Ordinary application fields stay
under fields, so a caller cannot overwrite scope_id by using a similarly named key.
Binding owns a snapshot; a child override retains unrelated parent fields and cannot
mutate its parent. Public error projection is separate from diagnostic classification.
The logger never automatically displays a foreign cause, diagnostic message or stack.

Delivery capacity counts waiting records; one additional record can be in flight.
Oversize/full/closed records increment dropped; encoding/sink failure increments
failed. Close stops acceptance, drains and flushes within its caller's deadline.
A deadline is not cancellation of an arbitrary blocking writer. No delivery retry,
file durability, audit or OTLP claim follows from stdout success.

Tests cover actual JSON/console/no-op paths, retained secret neighbors, filtering,
color/control escaping, blocked output, sink and flush failures, invalid timestamps,
oversize/closed records and safe projections. Root uses fixed stderr diagnostics for
delivery problems without changing a completed operation's business result.

## Rust mechanics

The facade owns a small Value enum, so arbitrary Display/Debug and serializer SDK
types do not become public field contracts. Non-finite Float values refuse encoding.
An owned tracing Dispatch installs the subscriber only for an emission; no global
subscriber or task-local provenance is required.

The adapter encodes an owned record into a reserved tracing event field. A custom
FormatEvent extracts it and emits JSON or the console line. This performs an extra
encoding step and is a deliberate initial cost, recorded in ADR 0006. A bounded
sync_channel and worker own Write; Arc shares state, atomics count outcomes, and a
Condvar supports close deadlines. Runtime Drop stops acceptance but does not claim
flush completion; callers explicitly call close.


JSON string escaping covers C0 controls but is not a complete console policy.
A final regression demonstrated literal DEL/C1 and Unicode line separators.
The console adapter now escapes them in messages, resources and encoded fields;
its own ANSI color is added separately. Structured JSON retains its native encoding.
