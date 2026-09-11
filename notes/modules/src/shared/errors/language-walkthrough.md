# Reading the Rust errors implementation

This walkthrough separates Rust syntax, ownership mechanics and the error policy
so each can be reviewed without treating them as one opaque abstraction.

## Origin and reading order

Written from the current error module and its public-API integration tests.
Language explanations were cross-checked with official Rust references. The
crate uses edition 2024 and was checked with Rust 1.86.0; edition and compiler
version are different settings.

| Read | What to notice | Review note |
| --- | --- | --- |
| [kind.rs](../../../../../src/shared/errors/kind.rs) | enum, derive, Self, associated constants, match and iterator closures | [Enums, Results and patterns](../../../../language/rust-enums-results-and-patterns.md) |
| [classification.rs](../../../../../src/shared/errors/classification.rs) | A supertrait, a default method and borrowed Classification | [Owned builders and borrowed views](../../../../language/rust-owned-builders-and-borrowed-views.md) |
| [failure.rs](../../../../../src/shared/errors/failure.rs) | Consuming builders, impl Into<String>, owned maps and a boxed source | [Traits and boxed sources](../../../../language/rust-traits-boxed-sources-and-context.md) |
| [context.rs](../../../../../src/shared/errors/context.rs) | Generic E, separate impl bounds, borrowed inner versus moved inner | [Traits and typed context](../../../../language/rust-traits-boxed-sources-and-context.md) |
| [public.rs](../../../../../src/shared/errors/public.rs) | Match guards and Option chains that produce an owned public value | [Owned builders and borrowed views](../../../../language/rust-owned-builders-and-borrowed-views.md) |
| [errors_spec.rs](../../../../../tests/errors_spec.rs) | #[test], domain enums, matches!, downcasts and expected values | [Specification and mutation technique](../../../../techniques/specification-tests-and-targeted-mutations.md) |

## Follow one failure

This complete review program uses the library's public API. It can be compiled
as an example target; the existing [errors example](../../../../../examples/errors.rs)
also demonstrates the same public refusal.

```rust
use n2f_rs::shared::errors::{Classified, Context, Failure, Kind, public_info};
use std::error::Error;

fn main() {
    let failure = Failure::new(Kind::Conflict, "email already registered")
        .with_type("account.email_taken")
        .with_field("email", "taken")
        .with_source(std::io::Error::other("PRIVATE database constraint"));
    let outcome: Result<(), Failure> = Err(failure);
    let contextual = outcome.map_err(|e| Context::new("register account", e));
    let refused = contextual.expect_err("this example is deliberately a refusal");
    let view = public_info(refused.classification());
    println!("{} {}", view.kind.as_str(), view.error_type.as_deref().unwrap_or_default());
    println!("{}", view.fields.get("email").map(String::as_str).unwrap_or_default());
    println!("{}", refused.inner().source().is_some());
}
```

Expected output:

```text
conflict account.email_taken
taken
true
```

Failure owns the metadata and source. Err moves it into an outcome; map_err moves
it into Context while preserving its type. classification borrows a view;
public_info copies only allowed data into an owned snapshot. The underlying
source remains available through the typed inner failure. expect_err is used
because this teaching example deliberately constructs Err, not as a general
request-handling policy.

## Compare the other implementations

Go keeps a reusable sentinel alive across copied occurrences; Rust retains an
enum case or a concrete E across ownership moves. TypeScript retains literal
discriminants while storing private data out of band. Rust's Box is used for a
heterogeneous diagnostic source, not as a requirement to erase every domain error.
The local [implementation guide](../../../../../src/shared/errors/README.md) maps all APIs.

## Evidence and limits

The tests use an external integration-test crate and import the public module.
That checks the exposed interface; it does not imply independent authorship.
E06 checks typed context/source retention, E10 checks merges, and the
[mutation report](mutations.md) records their sensitivity to selected faults.
These notes do not change the [behavior contract](../../../../../src/shared/errors/CONTRACT.md).

## Related reading

- [Metadata merges need a preserved-key assertion](../../../../techniques/metadata-tests-need-distinct-keys.md).
- [A Rust source chain does not supply classification](rust-adaptation.md).
