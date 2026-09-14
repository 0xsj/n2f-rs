# Keyed digest: text keys and a redacting Debug

**Origin:** implementing K01–K05 spec-first on 2026-09-12, with a mutation that
drops the purpose binding.

The contract wraps the key in the shared secret type, which in Rust is a
`String`, so the key is the text's UTF-8 bytes and nothing else. The first
shared vectors were generated from raw bytes and one of them (forty 0xab bytes)
could not be held by any `String`; a second byte-keyed constructor was tried and
dropped when the contract fixed keys as text and the vectors were regenerated.
The lesson kept: a fixture produced in a language whose strings hold arbitrary
bytes can encode inputs that a text-keyed language cannot construct.

`hmac` 0.12.1 was already in the offline registry through other crates and is
the only addition. `Hmac::new_from_slice` accepts any key length, so the 32-byte
minimum is this module's rule, checked before the MAC is built.

Verify signs first and compares with `subtle::ConstantTimeEq`; a tag of the wrong
length answers false before any comparison, as the contract asks. Signing with an
invalid purpose fails the same way in sign and verify, so a caller cannot get a
"false" out of a purpose typo.

`Digest` does not derive Debug; a hand-written impl prints a fixed marker. The
spec proves the key text never appears in that output.

**Limits:** three tests plus the vectors; no consumer exists yet. The
purpose-binding mutation (dropping the purpose and zero byte from the MAC input)
is caught by the vector test because every fixture tag was generated with the
binding.

**Used in:** src/shared/keyed/mod.rs and tests/keyed_spec.rs. See
[the contract](../../../../../src/shared/keyed/CONTRACT.md).
