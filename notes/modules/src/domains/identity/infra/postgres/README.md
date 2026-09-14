# Rust: identity store, markers for rollback and what the real database proved

**Origin:** implementing the identity PostgreSQL store (S01–S16) against a real
database on 2026-09-12, spec first, then six selected mutations.

The shared transaction wrapper commits on `Ok` and rolls back on `Err`, and after a
failed statement PostgreSQL aborts the transaction, so the wrapper's pre-commit
probe would turn a "duplicate" detected by a unique violation into a database
failure. Outcomes that must roll back (`stale`, `duplicate_email`) therefore travel
out of the callback as marker failures with private types and are translated to
values after the wrapper returns. The unique violation is recognized by its
constraint name before the shared `map` erases it; any other 23505 stays a generic
conflict.

Lock order is honored by never locking a session or challenge first. `resolve`
reads the digest's owner without a lock, then locks auth state, principal and
credential, then re-reads the session `FOR UPDATE`. The eight concurrent verify
consumers and the eight concurrent registrations both settle to exactly one
winner through row locks and the unique index alone; no advisory lock or retry.

The backward-time refusal cannot be left to the domain check inside `resolve`:
the domain also rejects backward time, but the store latches an observed expiry by
writing `revoked_at_ms`, so removing the explicit guard turns a clock glitch into a
persisted revocation. The spec's "rejected without a write" assertion is what
caught that mutation.

SQLx is used without its `uuid` feature, following the events adapter: UUIDs are
bound as text with a `::uuid` cast and read back as `id::text`; digests are
`bytea` bound as `Vec<u8>`. Versions are `u32` in the domain and `integer` in SQL;
the common ceiling keeps every value inside `i32`, and a negative column value is
reported as corruption rather than wrapped.

Sessions are not touched by `revoke_all`; the epoch column is the authority and
`resolve` rejects an epoch mismatch without writing, which is why an earlier
session still shows `revoked_at_ms` NULL after a password change.

**Limits:** developed against a scratchpad PostgreSQL 16.4 instance while the
PostgreSQL 18 check container could not start on a full Docker disk; once space was
freed, `tools/verify_identity_store.py` passed the same spec against PostgreSQL 18
in an owned disposable database. Uncertain commit is not reproduced here;
its classification belongs to the shared package (D04) and the store passes it
through. The events migration carries version 1 and this one version 2 in the
spec's ledger; root decides the real order.

**Used in:** src/domains/identity/infra/postgres/mod.rs, its migration and
tests/identity_store_integration.rs. See [the store contract](../../../../../../../src/domains/identity/infra/postgres/CONTRACT.md)
and the [application note](../../app/README.md).
