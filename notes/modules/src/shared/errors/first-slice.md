# The first errors slice: specification, execution and fault checks

The contract can guide multiple language implementations without pretending their
representations or verification evidence are identical.

## Origin

Implemented on 2026-09-11 from the local package documentation and the concrete
[scenario contract](../../../../../src/shared/errors/CONTRACT.md). Interfaces and placeholder bodies
were declared, tests were written against the public surface, then the initial
run was recorded before completing the behavior.

The initial run compiled and all nine integration tests failed against placeholder behavior.

## What changed

Rust uses Classified and a borrowed Classification view for public meaning.
Context<E> retains the typed case, while Failure owns metadata and a boxed
Error + Send + Sync source. No Clone bound is imposed on sources and no domain
registry is needed.

Review of the first aggregate fixture found that its supposed unknown item
was actually an explicitly classified Internal value. The fixture was corrected
to contain a real foreign error, and an assertion now checks that the summary's
retained aggregate still contains it. This was an ordinary test improvement
after the first implementation, not an independent blind test.

The aggregate fixture deliberately implements Classified with no summary. It
demonstrates an operation-owned contract; it does not establish a generic
inspector for arbitrary Rust error trees. Both selected valid mutations compiled,
were detected, and were restored before a green baseline rerun.

## Evidence and limits

Review removed an unnecessary assertion about vocabulary enumeration order.
The contract promises membership and stable names, so those are now checked
without preventing harmless reordering.

Nine public-API integration tests pass. Clippy checked all targets with warnings denied; rustdoc passed with warnings denied. The standalone errors example is included.

Selected faults: Exposing the Internal projection; dropping classification delegation in Context<E>. The final result was 2/2 valid mutations detected.
Compilation was checked separately; a build failure is not a test kill. Only the
mutated file was restored from bytes read immediately before each mutation, and
its hash was checked after restoration.

This was ordinary specification-first TDD with implementation visibility
available to the author. There was no separate test author, enforced information
barrier or claim of blind testing. The small mutation sample measures sensitivity
to those faults; it is not a completeness or correctness proof.

## Used in

- [The errors module](../../../../../src/shared/errors/mod.rs) and its public specification
  tests. The original model.rs was subsequently split by responsibility.
- [CONTRACT.md](../../../../../src/shared/errors/CONTRACT.md) for the exact first-slice scope.
