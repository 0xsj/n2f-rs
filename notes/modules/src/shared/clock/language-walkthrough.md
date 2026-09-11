# Rust clocks: interior mutability with checked transitions

**Origin:** C01–C06 on Rust 1.86.0 and the executable clock/ID example.

`SystemTime` names a wall instant; `Instant` measures elapsed time. A duration
obtained from `Instant` has no portable epoch. Its platform-dependent suspend
behavior is documented by [Rust](https://doc.rust-lang.org/std/time/struct.Instant.html).

`FakeClock::advance(&self, ...)` can update state because the fields sit behind
`Mutex<State>`. This is interior mutability: the shared reference permits only
the synchronization wrapper to control mutation. `let mut state = ...lock()`
creates a guard; dropping it releases the lock. Keeping wall and elapsed in one
state prevents a reader from observing half an advance.

`checked_add` returns `Option`. `ok_or(AdvanceError)?` converts overflow into a
small error and returns early. Both candidate additions happen before assignment,
so failure preserves both readings. `Duration` is unsigned: a negative advance
is excluded by the type rather than a runtime branch.

`Arc::clone` shares ownership of one fake; it does not duplicate its clock state.
Each test thread owns an Arc handle and the joins wait for all updates. Lock
poisoning is an unexpected panic path, not a shared application failure; no user
callback runs while the clock lock is held.

**Read:** [system](../../../../../src/shared/clock/system.rs), [fake](../../../../../src/shared/clock/fake.rs),
[public tests](../../../../../tests/clock_spec.rs), [worked example](../../../../../examples/time_and_ids.rs),
[mutation evidence](mutations.md).
