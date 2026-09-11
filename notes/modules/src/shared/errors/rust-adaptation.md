# A Rust source chain does not supply shared classification

Keeping a diagnostic cause and keeping a typed recovery case are separate
requirements when adapting the common error contract to Rust.

## Origin

Design review on 2026-09-11 while mirroring the Go draft. This finding comes from
reading the language API and reasoning about the proposed boundary, not from
testing an implemented n2f error module.

## What and why

Rust's [`Error::source`](https://doc.rust-lang.org/std/error/trait.Error.html#method.source)
exposes an optional diagnostic cause. The trait does not define our Kind or
public metadata. Consequently, a generic source walker alone cannot establish
the blueprint's classification contract. A source chain also does not enumerate
independent batch failures.

The first implementation keeps domain cases typed and exposes classification through an
explicit mapping or a small trait. A context wrapper must preserve both the
recoverable case and its diagnostics. Erasing every operation error into a common
container immediately would make local handling less direct; importing every
domain enum into the shared module would reverse the dependency direction.

Go's preserved sentinel identity becomes preserved typed case handling here.
There is no reason to introduce pointer identity, Clone requirements on arbitrary
causes, or a universal domain-error registry just to match the Go method names.

## Gotchas

A boxed diagnostic source and a typed wrapper have different ownership and
inspection costs. The first slice uses owned Failure sources and Context<E> to
retain the local case. [Its tests and findings](first-slice.md) cover that
choice. It does not claim automatic classification of arbitrary foreign errors.

## Used in

- [Draft errors module contract](../../../../../src/shared/errors/mod.rs), especially typed
  cases, source handling, classification and aggregate outcomes.
