# Rust enums make alternatives explicit; patterns decide what each means

A closed enum captures the cases we own, while Result and Option keep failure
and absence separate in ordinary control flow.

## Origin

Review of Kind, the operation-owned error enum used in the tests, the registration
example, and the public-projection match. These snippets are excerpts using the
surrounding Kind, Failure and Context definitions.

## Read Kind and its implementation

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Internal,
    Invalid,
    Conflict,
}
```

`enum` declares alternatives; `Kind::Conflict` names a variant through a path.
Unlike an integer-backed Go vocabulary, ordinary safe callers cannot construct
an arbitrary Kind number. The compiler still cannot decide what an external
string should mean: the parser explicitly returns None for unknown names.

`#[derive(...)]` asks the compiler to generate trait implementations. Debug
supports diagnostic formatting, PartialEq/Eq support equality, Clone supplies
explicit cloning, and Copy permits implicit copying of this small value. These
traits apply to this type; they do not make all error payloads copyable.

```rust
impl Kind {
    pub const ALL: [Self; 3] = [Self::Internal, Self::Invalid, Self::Conflict];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::Invalid => "invalid",
            Self::Conflict => "conflict",
        }
    }
}
```

This shortened vocabulary illustrates the syntax; the implementation has ten
variants. `impl Kind` contains associated items. Uppercase Self means the current
implementing type; lowercase self is a receiver value. `[Self; 3]` is an array of
three items, unlike a Vec that can grow. `pub` exposes an item through the module's
public path.

The match is an expression. Each `pattern => expression` supplies a result;
without a catch-all, adding a variant requires updating the arms. The returned
string literals have static lifetime and require no owned String per call.
There is no implicit contract that enum positions are public wire values.

## Read the parser's iterator chain

```rust
Self::ALL.into_iter().find(|kind| kind.as_str() == name)
```

The array produces an iterator of Kind values. The closure `|kind| ...` is the
predicate passed to find; find borrows each candidate for inspection. Method-call
adjustments and Kind's Copy implementation allow the value-receiver as_str call.
The result is an Option<Kind>: Some for a match, None if no candidate matches.

## Result, Option and the unit type

```rust
let outcome: Result<(), Failure> = Err(failure);
let contextual = outcome.map_err(|e| Context::new("register account", e));
```

The type after `:` annotates the binding. Result has a success type and an error
type. `()` is the unit type: success carries no additional payload. Err carries
a failure; `map_err` transforms only that branch and leaves Ok intact. Here it
moves the failure into Context instead of copying it.

`Result<Option<User>, Failure>` distinguishes a failed lookup from a successful
lookup with no user. `Ok(None)` and `Err(...)` are different outcomes. Similarly,
None returned by the classification protocol means an existing error has no shared
classification; the caller is already on an Err path when projecting it.

## Pattern syntax used by the tests and projection

- `Some(view) if view.kind != Kind::Internal => ...` combines an enum pattern
  with a guard. A guard that fails lets a later arm handle the value.
- `_ => ...` is a catch-all pattern. It is not the same construct as an inferred
  lifetime written `'_`.
- `RegisterError::EmailTaken(Failure)` is a tuple variant carrying data, not an
  exception subclass. Matching the variant preserves domain vocabulary.
- `matches!(value, Pattern)` is a macro producing a boolean; `assert!` checks a
  boolean, while `assert_eq!` compares values and reports them on failure.
- `#[test]` registers a test function. `println!` and `assert_eq!` use `!` because
  they are macros, not ordinary function calls.

## Gotchas

An exhaustive match protects the enum's cases, not the business meaning assigned
to them. A compiler cannot tell whether Conflict was the correct classification.
The scenario tests still need to establish that behavior.

## Used in and related

Used in owned domain error cases, explicit success/absence/failure outcomes and
public boundary translation. Continue with [owned builders and borrowed views](rust-owned-builders-and-borrowed-views.md).
The standard [Option reference](https://doc.rust-lang.org/std/option/enum.Option.html)
describes the container and iterator-like combinators used here.
