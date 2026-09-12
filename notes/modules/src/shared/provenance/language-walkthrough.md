# Provenance: implementation walkthrough

ScopeSnapshot now nests WorkSnapshot. This makes the different lifetimes visible:
retry preserves the whole work snapshot and changes execution fields. Prepare
produces work without consuming time or IDs; execute supplies attempt and executor.
A supplied correlation hint grants no authority and cannot supply actor, tenant,
scope ID, attempt, origin or known depth.

Pure validation runs before clock/ID effects. The factory reads its clock once,
then calls the generator once; the generator can independently read its own clock.
Clock rollback changes wall start time without changing retry identity rules.
Counter overflow refuses before effects; failed ID generation retains its original
failure. A supplied explicit cause cannot hide a generated parent-scope collision.

The first core tests were written before implementations. The later collision
regression was added with implementation visibility during review. P20–P22/P24
remain obligations of future transport/envelope/audit owners, not implemented
persistence or socket guarantees.

## Rust mechanics

Enums close ActorKind/ReferenceKind/Origin vocabularies. Owned strings and snapshots
derive Clone deliberately; Id and Reference can be Copy. Scope/WorkContext have no
Default or public fields, while snapshots are editable owned data that restoration
validates. Borrowed LinkSet values cannot mutate the set.

Factory<C,G> accepts Fn clock and FnMut generation. Methods take &mut self, which
requires exclusive access to generator effects. checked_add plus transpose handles
optional depth while keeping overflow a Failure. ? propagates the existing Failure
and boxed diagnostic source without imposing Clone on arbitrary errors.

