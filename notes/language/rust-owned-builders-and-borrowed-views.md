# Rust can move the error and borrow its classification

Consuming builders preserve ownership without cloning the cause, while borrowed
views let callers inspect data without taking it away.

## Origin

Review of Failure's builders, Classification's lifetime, PublicInfo's owned
projection, and the metadata ownership tests.

## Read a consuming builder

```rust
pub fn with_type(mut self, value: impl Into<String>) -> Self {
    let value = value.into();
    self.error_type = if value.is_empty() { None } else { Some(value) };
    self
}
```

This method-body excerpt uses surrounding Failure fields. `self` is taken by
value, so the method consumes the old value. `mut self` permits changing that
owned value inside the method; it does not grant access to an aliased original.
The final `self` expression returns it. Adding a semicolon would discard that
expression's value and leave the block returning unit, which would not match
Self here.

`impl Into<String>` accepts an argument whose type can become String. A string
slice is converted to owned text; an existing String can be moved through the
conversion. `.into()` uses the expected target type to select the conversion.
The if/else is also an expression, producing None or Some(value).

```rust
let base = Failure::new(Kind::Invalid, "invalid input");
let enriched = base.with_field("email", "required");
// base cannot be used again here: with_field consumed it.
```

The builder does not require the caller's binding to be mutable because it takes
ownership. In contrast, an `&mut self` method would borrow exclusive access and
leave ownership with the caller. These move/borrow distinctions are explained in
the [Rust ownership chapter](https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html).

## Read a borrowed classification frame

```rust
pub struct Classification<'a> {
    pub kind: Kind,
    pub error_type: Option<&'a str>,
    pub message: &'a str,
    pub fields: Option<&'a BTreeMap<String, String>>,
}
```

`'a` names a lifetime relationship among the borrowed fields. The view may not
outlive the referenced data; the annotation does not keep that data alive or
extend its lifetime. Kind is copied, while text and the map are borrowed.
`&str` is a borrowed string slice; String owns its storage.

In `fn classification(&self) -> Option<Classification<'_>>`, `'_` lets the
compiler infer the lifetime tied to the self borrow. This is appropriate for a
short-lived inspection view. The [lifetime chapter](https://doc.rust-lang.org/book/ch10-03-lifetime-syntax.html)
explains these relationships and elision.

Classification derives Clone and Copy because its payload consists of copyable
kinds and references. Copying the view does not copy the referenced map. Failure
owns Strings, maps and a dynamic source and intentionally does not implement
Clone; arbitrary sources need not be cloneable.

## Turning borrowed data into an owned public snapshot

```rust
view.error_type
    .filter(|value| !value.is_empty())
    .map(str::to_owned)
```

Starting with Option<&str>, filter removes an empty identifier and map converts a
remaining slice into String. The function path `str::to_owned` is passed as the
mapping operation; a closure such as `|text| text.to_owned()` expresses the same
intent here.

```rust
view.fields.cloned().unwrap_or_default()
```

For Option<&BTreeMap<...>>, cloned produces an owned map when present, and
unwrap_or_default supplies an empty map when absent. It does not panic like
unwrap on None. PublicInfo consequently owns its strings and fields independently
of the failure. See the [Option combinator reference](https://doc.rust-lang.org/std/option/enum.Option.html).

## Map ownership and merge semantics

`BTreeMap<String, String>` stores ordered keys and owned string values. The ordered
container makes inspection predictable; the shared contract does not promise a
field iteration order or prescribe this map type to the other languages.

`with_fields(values: BTreeMap<...>)` takes the incoming map by value, then extends
the existing map. Incoming entries win collisions, while unrelated existing keys
remain. Replacing the destination with values would satisfy ownership but break
the merge promise. The mutation test demonstrated that distinction.

`details(&self) -> &BTreeMap<...>` exposes an immutable borrow for internal
inspection. Callers can clone it for their own modifications. This is different
from returning a mutable reference to the failure's map.

## Gotchas

An immutable borrow protects access through that reference; it is not a blanket
claim that every possible Rust type has no interior mutability. The metadata here
is simple owned strings. The diagnostic source remains a separate abstraction
with its own implementation and possible internal state.

## Used in and related

Used in fluent builders, diagnostic carriers and boundary snapshots that outlive
an inspection borrow. Continue with [traits, boxed sources and typed context](rust-traits-boxed-sources-and-context.md).
