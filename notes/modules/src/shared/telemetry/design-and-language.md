# Trace identity does not replace work identity

**Historical design note:** the implementation and verification now live in this
module's README and root HTTP verification notes; future tense below records the
original plan, not current status.

**Origin:** review of the implemented provenance/logger notes and W3C/OpenTelemetry
documentation on 2026-09-11. This is design reasoning, not measured adapter behavior.

Provenance gives an execution a Scope and lets logical work survive retries.
Tracing may sample that execution out while its work still exists. Reusing a UUID
as trace identity would couple these lifetimes and conceal the possibility of
valid non-recording context. A logger field also cannot create an active SDK span.

The first leaves therefore validate an owned trace projection and a finite outcome
vocabulary. HTTP supplies the consumer that makes lifecycle/export decisions concrete.
A handled conflict can be refused without setting a server span to Error.
Delivery diagnostics are independently lossy and must not be mistaken for audit.

## Rust techniques to test

Use an exhaustive Outcome enum and Option<TraceRef> for absence. Private fields
and no Default implementation keep invalid trace values out of ordinary
construction. An owned snapshot can clone its strings without giving mutable
access to the source; borrowed projections must not outlive their owner.

The future SDK adapter can own runtime resources without exposing them through
these value APIs. Existing tracing log formatting is a separate adapter; crate
imports alone do not connect it to distributed spans.

**Used in:** the [contract](../../../../../src/shared/telemetry/CONTRACT.md) and
[native leaf plan](../../../../../src/shared/telemetry/API.md). Runtime checks and selected mutations
remain pending.

The Rust leaf suite covers zero, malformed, uppercase and invalid-length IDs plus
every exact outcome. Private fields and the exhaustive enum keep construction
idiomatic without a default invalid value.
