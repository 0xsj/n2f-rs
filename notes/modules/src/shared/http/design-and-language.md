# Completion has several observations, but one owner

**Historical design note:** the implementation and verification now live in this
module's README and root HTTP verification notes; future tense below records the
original plan, not current status.

**Origin:** design review of the foundations, public error APIs and HTTP/runtime
documentation on 2026-09-11. These are prospective tests, not observed guarantees.

A handler result, a committed status and a response-body completion describe
different moments. If a handler succeeds and the peer disconnects, telemetry must
retain what it knows without claiming rollback or client receipt. Several callbacks
may observe the same end; putting the completion guard in the test recorder would
hide a duplicate-emission bug in production.

Safe error projection is a second boundary: unknown failures stay failures even
when their native representation is absent-looking. In TypeScript, throw undefined
cannot become a successful empty response; the Result branch carries failure
presence. Rust public_info(None) likewise means an unclassified failure. Go's nil
error means absence. Matching scenarios need explicit native inputs, not identical
null conventions.

## Rust techniques to test

Use an exhaustive enum for outcome/termination and Option for unknown status or
missing trace context. An owned value with private fields prevents a caller from
constructing all-zero IDs through a struct literal. Borrowing a completed snapshot
cannot mutate the source.

An entered tracing span guard must not be retained across await; it can associate
another task's execution with the wrong span. Instrument the future or use the
selected adapter's scoped activation. A Drop guard can record abandonment, but
cannot await flush or turn abandonment into success.
[tracing's async guidance](https://docs.rs/tracing/latest/tracing/struct.Span.html#in-asynchronous-code)
informs this plan. Existing logger Dispatch ownership and future SDK span ownership
must be connected deliberately; neither the connection nor a dropped-body test
has been implemented.

**Used in:** [H01–H14](../../../../../src/shared/http/CONTRACT.md) and the
[portable completion shapes](../../../../../src/shared/http/SHAPES.md). The diagnostic process
must verify actual writer/body hooks before claiming adapter parity.

## An observed documentation check

The first rustdoc run with warnings denied treated a bare `<kind>` in the included
Markdown table as an unclosed HTML element. Wrapping the URI template in inline
code fixed the warning in the common contract; rustdoc then passed. include_str!
feeds Markdown to rustdoc rather than treating it as an opaque text attachment.
This verifies documentation rendering rules, not any HTTP behavior.
