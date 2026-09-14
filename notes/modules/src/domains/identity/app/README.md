# Rust: application operations over one generic port bundle

**Origin:** implementing the identity application contract (U01–U16) against the
recording fakes on 2026-09-12, resumed after the first attempt was cut off with
the spec written and nothing implemented.

Each operation is generic over a single type `P` bounded by every trait it
consumes, rather than one type parameter per port. The spec constructs every
operation from one fake value, and a real root can do the same with one adapter
bundle or a struct of references. The cost is that a bound list names the
operation's dependencies twice, once on `new` and once on the method impl; the
gain is that adding a port to an operation is a bound change, not a signature
change at every call site.

Port futures are `impl Future<Output = …> + Send` in the trait (return-position
impl Trait in traits, edition 2024). No boxing crate was needed, and the fakes
return `async move { r }` with the scripted value computed synchronously so the
recorded call order is the order the operation issued the calls, not the order
the futures were polled.

`Clock` and `TokenCodec` are declared once in `command` and consumed by `query`
through `super::command`. The contract says each package declares the ports it
consumes; redeclaring the same two traits in `query` would have forced the fake
to implement both copies for no behavioral difference. This is a deliberate
deviation, recorded in STATUS.

`domain::MAX_TIME_MS` and `MAX_VERSION` became `pub` because the spec and the
application both need the common bounds; they were `pub(super)` while only the
domain used them. Config validation reuses them, and the identity mutation
harness substrings in `tools/mutations` were unaffected.

`LoginResult` carries the session secret, so it implements `Debug` by hand with
the secret redacted: the spec's `unwrap_err` needs `Debug` on the success type,
and a derived `Debug` would have printed the token in a test failure message.

The first clippy run found the spec, not the library: a `MutexGuard` kept alive
across an `.await` by an explicit `drop` still trips `await_holding_lock`, so the
guarded assertions are scoped in blocks. A scripted enum that is `Copy` should not
be cloned, and a boxed `Fn` list needs a type alias to stay readable.

**Limits:** every port is a fake. Nothing here proves a transaction, a lock order,
a real hash duration, real mail or a real limiter; those are stage 5–7 claims. The
six selected mutations (enumeration leak, duplicate replacement, mail after an
uncertain commit, verification without password proof, stale login as success,
private data in an event) were each caught by a named test, in place with a
hash-verified restore; the isolated-copy harness belongs to the shared tooling.

**Used in:** src/domains/identity/app/{mod.rs, command.rs, query.rs} and
tests/identity_app_spec.rs. See the [contract](../../../../../../src/domains/identity/app/CONTRACT.md)
and the [auth leaves walkthrough](../domain/auth-leaves-walkthrough.md).
