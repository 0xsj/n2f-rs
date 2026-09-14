# Entropy moved out of id when a second consumer appeared

**Origin:** stage 3 of the auth build on 2026-09-12. Token issuance and password
salts need the same injected OS randomness that UUIDv7 generation uses.

Decision 0004 confined `getrandom` to a private adapter inside `id`. That was
right with one consumer; a token codec importing `id` to obtain randomness would
have named the wrong dependency. `Entropy`, `EntropyError` and `OsEntropy` now live
in `shared::entropy`, and `id` re-exports them so its consumers and tests are
unchanged. `getrandom` is still confined to one struct; only its module moved.

The alternative was a second private adapter inside identity. It would have kept
`id` untouched at the cost of two owners for the same third-party dependency and
two places to change if the entropy policy ever did.

**Limits:** a module move only. The id suite passed unchanged; no new entropy
behavior or failure mode was added.

**Used in:** src/shared/entropy/mod.rs, src/shared/id/v7.rs and the identity
adapters. See [id language notes](../id/language-walkthrough.md).
