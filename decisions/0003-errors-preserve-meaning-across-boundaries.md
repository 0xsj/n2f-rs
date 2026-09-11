# 0003 — Errors preserve meaning across boundaries

**Status:** Accepted · **Date:** 2026-09-11

## Context

The blueprint needs comparable failure behavior while keeping domains independent
of transports, loggers and storage libraries. The package documentation defines
meaning; the [first-slice contract](../src/shared/errors/CONTRACT.md) made the interface and
scenario choices concrete before completing implementation. This record
consolidates those choices after the first TDD pass.

## Decision

Use the same ten categories, optional domain-owned Type identifiers, explicit
public projection and retained private diagnostic causes. Annotation preserves
meaning; translation supplies a complete new public frame. Unknown classification
stays distinct from an Internal presentation fallback.

Rust uses Classified and a borrowed Classification view for public meaning.
Context<E> retains the typed case, while Failure owns metadata and a boxed
Error + Send + Sync source. No Clone bound is imposed on sources and no domain
registry is needed.

Metadata belongs to each value. A source replacement is explicit and never
simulated with an aggregate. Aggregates require an explicit summary or remain
unclassified. Retry decisions, logging and wire mappings stay with their owners.

## Alternatives

A universal representation across languages would make local typed handling less
natural. Domain-only errors without shared classification would move a growing
catalog of domain cases into generic adapters. Returning raw exceptions or
diagnostic strings would couple callers to implementation details and disclosure
choices. Implicit reclassification during annotation would let context change
recovery meaning.

## Consequences

The leaf has no new dependency and can be tested without infrastructure. Each
language needs a small explicit projection/mapping boundary. Stable Type values
become a compatibility commitment when published. HTTP and WebSocket adapters
still need their own contracts and tests.

## Verification

Nine public-API integration tests pass. Clippy checked all targets with warnings denied; rustdoc passed with warnings denied. The standalone errors example is included. Two targeted valid fault injections were detected and sources
restored. See the [TDD note](../notes/modules/src/shared/errors/first-slice.md) and
[status](../STATUS.md) for the initial failures and evidence limits.
