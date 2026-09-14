# Rust: canonical base64url is partly the library's job

**Origin:** implementing the token codec contract (T01–T08) on 2026-09-12,
spec-first, then in-place mutations.

The base64 crate's default engines already refuse non-zero trailing bits
(`decode_allow_trailing_bits` is false) and refuse padding under
`URL_SAFE_NO_PAD`. The contract's re-encode equality check is therefore defense
in depth in this build: removing it alone changes no observable behavior, and the
named noncanonical_accept mutation only bites when the decoder is also made
lenient. The check stays because the contract, not the crate configuration, is
the authority on canonical form, and a future engine change must not silently
widen acceptance.

`Issued` deliberately has no `Debug`. The spec asserts that at compile time with
an inherent associated const on a `T: Debug` wrapper that shadows a blanket trait
const; the assertion is a `[(); bool as usize]` array length so clippy does not
fold it into `assert!(true)`.

Two contract refusals are unreachable in Rust and the Result shapes exist for
parity with Go and Node: `TokenPurpose` is a closed enum, so an invalid purpose
cannot be passed, and `Codec::new` cannot be handed a missing capability.
Entropy failure is the only refusal `issue` can produce, and the test proves a
partial fill is discarded rather than used.

**Limits:** no cookie, header, storage lookup or comparison exists here; the
digest is computed, not matched.

**Used in:** src/domains/identity/token_codec/mod.rs and tests/token_codec_spec.rs.
See the adapter [contract](../../../../../../src/domains/identity/token_codec/CONTRACT.md).
