# Keep the local error generic and the diagnostic source dynamic

A generic context retains a caller's concrete error type, while a boxed trait
object can hold an adapter's unknown concrete source type.

## Origin

Review of Classified, Failure's source storage and Context<E>. The classification
and cause-chain tests exercise both forms of abstraction.

## Read the classification trait

```rust
pub trait Classified: StdError {
    fn classification(&self) -> Option<Classification<'_>> {
        None
    }
}
```

`trait` defines a protocol. `: StdError` is a supertrait bound: an implementer must
also implement the aliased standard Error trait. That standard trait requires
Debug and Display. The default method returns None, so a type can explicitly
participate while declaring that it has no shared classification.

`use std::error::Error as StdError` renames an import locally; it creates no new
type. `impl Classified for Failure` supplies behavior for a particular type.
Trait methods must be brought into scope where method-call syntax needs them,
which is why consumers import Classified alongside Failure.

The standard source protocol supplies diagnostic chaining, not this custom
classification. A `dyn StdError` cannot be assumed to have a Classified method.
See the [standard Error trait](https://doc.rust-lang.org/std/error/trait.Error.html).

## Decode the source field one piece at a time

```rust
Option<Box<dyn StdError + Send + Sync + 'static>>
```

| Syntax | Responsibility |
| --- | --- |
| `Option<...>` | A failure may have no diagnostic source |
| `Box<...>` | Own storage for a value accessed through a pointer |
| `dyn StdError` | Use an error trait object; concrete source types can differ |
| `Send` | Permit transferring ownership between threads |
| `Sync` | Permit sharing references between threads |
| `'static` | The stored source cannot contain a borrow that expires sooner |

The static bound does not mean the source lives forever; it can be dropped with
its owner. Owned strings can satisfy it. The bounds also do not spawn a task or
make the operation retryable. Classified itself deliberately has no Send/Sync
supertraits, leaving domain error types less constrained.
The standard [Send](https://doc.rust-lang.org/std/marker/trait.Send.html) and
[Sync](https://doc.rust-lang.org/std/marker/trait.Sync.html) references define
these thread-transfer and shared-reference guarantees.

The source constructor takes `impl StdError + Send + Sync + 'static`: that
argument is generic. `Box::new(source)` stores the concrete value, which is then
coerced into the dynamic source field. The [trait-object chapter](https://doc.rust-lang.org/book/ch18-02-trait-objects.html)
explains generic versus dynamic dispatch.

## Decode source() without confusing its two lifetimes

```rust
fn source(&self) -> Option<&(dyn StdError + 'static)> {
    self.source
        .as_deref()
        .map(|source| source as &(dyn StdError + 'static))
}
```

`as_deref()` borrows through the optional Box. The cast presents the error without
requiring its caller to see the extra Send/Sync bounds. It does not copy the
source. The returned reference borrows from self; the `'static` bound describes
the underlying error's captured borrows, not a promise that this reference itself
can be kept forever.

Display prints this failure's own diagnostic text. Source remains independently
inspectable, avoiding a requirement to flatten the entire chain into one string.
Neither Display nor Debug is the public projection.

## Read the generic context

```rust
pub struct Context<E> {
    operation: String,
    inner: E,
}
```

E preserves the concrete inner type. `inner(&self) -> &E` borrows it;
`into_inner(self) -> E` consumes the wrapper and moves it back out. This lets a
caller still match its domain enum rather than decoding a string label.

The separate blocks `impl<E> Context<E>`, `impl<E> Display for Context<E>` and
`impl<E: Classified + 'static> Classified for Context<E>` apply different bounds
only where needed. A context can store E without requiring every possible method
or trait up front. Its classification implementation delegates to the inner
error; the selected mutation replaced that delegation with None and the tests
caught the lost meaning.

`downcast_ref::<RegisterError>()` in the tests asks a dynamic error for a borrowed
reference to a specific concrete type. `::<...>` supplies an explicit generic
argument. The result is optional: a different concrete source type does not
magically turn into the requested type.

## Modules and exports

`mod failure;` declares an implementation module. `pub use failure::Failure;`
re-exports its public type from the parent module. `super::Kind` resolves through
the parent module. Splitting files therefore need not change consumer imports.
`//!` documents the containing module; `///` documents the following item.

## Gotchas

Boxed source types do not automatically implement Clone, and not every standard
error implements the shared classification trait. Those are deliberate boundaries,
not gaps to fill with an automatic registry. Downcasting does not replace explicit
classification or operation-owned handling of aggregates.

## Used in and related

Used where typed domain outcomes meet heterogeneous adapter diagnostics. Review
[owned builders and borrowed views](rust-owned-builders-and-borrowed-views.md)
for the ownership that makes source retention possible without cloning.
