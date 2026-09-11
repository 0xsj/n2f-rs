# An already boxed source needs an explicit builder

**Origin:** the ID module's first entropy adapter, Rust 1.86.0 compiler output,
E11 and the I06 diagnostic downcast scenario.

The old `with_source(source: impl Error + Send + Sync + 'static)` accepts a sized
concrete error and boxes it. The entropy port already owns
`Box<dyn Error + Send + Sync>`. Feeding that into the generic bound failed: the
standard Box Error implementation does not make this unsized trait-object case
satisfy the builder's bound. A cast cannot repair that ownership mismatch.

`with_boxed_source` accepts the existing box directly. It replaces the old source
and leaves classification and metadata untouched. No second wrapper sits between
Failure::source and the original error, so a downcast can recover an io::Error.
Using a string instead would have discarded the diagnostic type.

E11 tests source replacement, direct downcast, public projection and retained
private details. I06 exercises the actual entropy consumer. A mutation dropping
the boxed source compiled and failed E11. The combined Cargo run stops after the
first failing integration binary; its mutation report therefore names E11 only.

See [the builder](../../../../../src/shared/errors/failure.rs),
[contract](../../../../../src/shared/errors/CONTRACT.md),
[public tests](../../../../../tests/errors_spec.rs),
[entropy boundary](../../../../../src/shared/id/v7.rs) and
[mutation evidence](boxed-source-mutations.json).
