# Rust: auth leaves, normalization and what the type system does not check

**Origin:** implementing the auth leaf specs (A01–A09) against the refusing
scaffolds on 2026-09-12, with six selected mutations run afterwards.

NFC normalization is not in `std`. The `unicode-normalization` crate (pinned
0.1.25, already in the local registry through other dependencies) supplies `nfc()`
over `chars()`. The raw input ceiling is checked on bytes before normalizing and
again on the normalized bytes, because NFC composition can change length in either
direction; the enrollment scalar count is taken after normalization only.

The malformed-UTF-8 fixture that Go and Node exercise cannot reach the Rust leaves:
`SecretString` wraps a `String`, which is valid Unicode by construction. The Rust
suite therefore has no malformed-input test; that is a language boundary moved
into whoever decodes bytes into a `String`, not a missing check.

`Id` has no zero value in Rust, so the credential, session and challenge
snapshots omit the zero-ID refusals the Go tests carry. `Session`, `Challenge` and
`AuthState` have private fields and no public unchecked constructor, so the
"forged zero value" checks are also absent by design rather than untested.

`CredentialSnapshot` deliberately derives neither `Clone` nor `Debug`: the hash is
a `SecretString`, which has no `Clone`. Snapshot copying is an explicit field-wise
function that re-wraps the revealed hash text, keeping the disclosure point visible.
An `Option<i64>` copies by value, so present-zero and absent timestamps never alias
caller storage; the ownership assertions in the spec pass without defensive code.

Refusal classification follows the leaf spec: a clock outside the common range, a
non-positive or overflowing idle TTL and revoke/invalidate before the stored
activity/issue time are `Invalid` (`session_invalid`, `challenge_invalid`), while
revoked, expired, backward-time, wrong-purpose, wrong-version and already-terminal
use are `Unauthenticated` (`session_rejected`, `challenge_rejected`). `Touch` calls
`check` before validating the TTL, so an expired session reports rejection rather
than a TTL complaint. Equality at either deadline is expired; the mutation that
turns `>=` into `>` on the idle deadline is caught by the issue/activity test.

Common bounds (`MAX_TIME_MS`, `MAX_VERSION`) now live once in the module root as
`pub(super)` helpers. The principal code still spells the literals it had; it was
left untouched so the existing principal mutation harness keeps matching.

**Limits:** six selected mutations (expiry equality, purpose confusion, replay,
stale version, revocation check, secret disclosure) were each caught by a named
test, in place with hash-verified restoration. No database, hashing, token
generation or process wiring exists for these values.

**Used in:** email.rs, password.rs, token.rs, credential.rs, auth_state.rs,
session.rs, challenge.rs and tests/auth_leaves_spec.rs. See
[authentication boundaries](authentication-boundaries.md).
