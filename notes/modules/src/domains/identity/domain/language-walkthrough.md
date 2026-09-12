# Rust: closed variants and owned restoration

**Origin:** implementing the external principal spec tests.

`enum Kind` and `enum Status` restrict callers to named variants. A later wire or SQL
decoder must reject unknown strings before constructing them. Shared `Id` already
rejects zero; these invalid states cannot be represented through the safe public API.

`Result<Self, Failure>` returns either a principal or a safe classified failure.
`&self` borrows the current value; transitions clone the snapshot, modify the owned
copy, and return a new principal. Snapshot owns its String, so `clone()` also copies
the name allocation. `Copy` is derived only for the small enum values.

The private state field and validated constructors maintain invariants. There is no
public default constructor. Restore takes ownership of a snapshot, validates it,
then preserves it. The version uses u32 but retains the common signed-32-bit ceiling.

`String` is valid UTF-8; `chars()` yields Unicode scalar values. Invalid UTF-8 is a
decoder concern here, while Go/TypeScript can represent malformed string data.

One early compilation failure came from placing `pub mod identity;` before existing
`//!` inner docs. Inner docs must precede module items; this was a wiring failure,
not evidence of a behavioral test catching a domain defect.

**Used in:** domain/mod.rs and tests/identity_domain_spec.rs.
See [invariants and evidence](README.md).

