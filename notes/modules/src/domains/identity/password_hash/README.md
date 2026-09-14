# Rust: Argon2id behind a semaphore, and what the runtime hides

**Origin:** implementing the password hashing adapter contract (H01–H10) on
2026-09-12, spec-first against an absent module, then seven in-place mutations.

The argon2 crate's PHC parser is not used. `password_hash::PasswordHash::new`
would accept any well-formed record and only refuse parameters when `Params`
is built, after the string is trusted. The contract wants the exact allowlist
decided before any allocation, so `parse` compares the algorithm, version and
parameter fields as literal strings and decodes salt and tag with
`STANDARD_NO_PAD` plus a re-encode equality check. The crate then only computes
a tag from fixed `Params`; its `password-hash` feature is currently enabled but
unused and could be dropped when the dependency list is next reviewed.

`#[tokio::test]` defaults to a current-thread runtime. The admission scenarios
block the test thread on a std channel while waiting for a spawned task to start,
which deadlocks there: the task never gets the thread. Those three tests run on
`flavor = "multi_thread"`. The lesson generalizes: a blocking wait inside an
async test is only valid when another worker can drive the future.

The queue counter is decremented by a `Drop` guard, not after the await. An
abandoned waiter (an aborted task, a caller's own timeout dropping the future)
never reaches the code after `acquire_owned`, so without the guard the queue
would leak a slot per abandonment until saturation refused everyone. The test
aborts a queued task and then proves a fresh caller can still queue.

The semaphore permit moves into the `spawn_blocking` closure. That is what makes
"released after panic" true: unwinding the blocking task drops the permit. The
`JoinError` becomes Internal `identity.hash_failed`; the admitted work is never
canceled by the caller, so `queue_wait` bounds only the wait, as H07 requires.

The dummy record constant was first typed from a guess and the initial H05 test
still passed, because `verify_absent` discards the comparison. The spec now pins
`dummy_record()` to the fixture and feeds the fixture's own tag through the work
seam, which is also what makes the dummy_leaks mutation observable.

**Limits:** interoperability is proven by the three fixture hashes and the verify
table; real timing, memory pressure and the Argon2 memory bound under concurrent
load are not measured here. No application operation calls this adapter yet.

**Used in:** src/domains/identity/password_hash/mod.rs and tests/password_hash_spec.rs.
See the adapter [contract](../../../../../../src/domains/identity/password_hash/CONTRACT.md).
