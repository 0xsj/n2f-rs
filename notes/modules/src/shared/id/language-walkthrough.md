# Rust IDs: value traits, closure capabilities and boxed sources

**Origin:** I01–I09 and the compiler failure encountered at the first entropy boundary,
verified on Rust 1.86.0.

`Id([u8; 16])` is a tuple struct with a private field. Deriving Copy/Eq/Ord/Hash
provides value behavior without exposing construction. FromStr supplies the
`text.parse::<Id>()` convention; Display supplies formatting. The parser reads
ASCII bytes, avoiding slicing an arbitrary UTF-8 string at a character boundary.

`V7<C, E = OsEntropy>` is generic over its wall closure and entropy capability.
`C: Fn() -> SystemTime` permits repeated shared calls; Entropy's blanket
implementation accepts `FnMut`, so a deterministic source may own changing state.
`new_id(&mut self)` makes the generator transition exclusive. Sharing across
threads uses caller-owned `Arc<Mutex<_>>`; the clock fake already has interior
locking and can be captured through an Arc. No generator Clone duplicates counters.

`Option<(u64, u16)>` distinguishes no successful ID from timestamp zero. `as_millis`
returns u128, so range validation happens before narrowing to u64. Local timestamp,
counter and random bytes are prepared before committing the state tuple. A `?`
return from entropy therefore preserves the previous successful state.

The entropy port owns `Box<dyn Error + Send + Sync>`. Passing that box to the old
`with_source(impl Error)` builder failed its Sized-related Error bound. The new
`with_boxed_source` accepts the existing box directly and preserves a direct
source downcast; [the error note](../errors/boxed-sources.md) records the finding.
This is why a real next consumer can improve a leaf's interface.

getrandom 0.4.1 is confined to OsEntropy. The host dependency tree contains cfg-if
and libc; Cargo.lock also records target-specific packages that the macOS checks
do not build. Other targets, including WASI entries with higher MSRVs, remain
unverified. A lockfile entry is not proof that every target was compiled.

**Read:** [value](../../../../../src/shared/id/value.rs), [generator](../../../../../src/shared/id/v7.rs),
[sequence](../../../../../src/shared/id/sequence.rs), [public tests](../../../../../tests/id_spec.rs),
[worked example](../../../../../examples/time_and_ids.rs), [mutation evidence](mutations.md).
