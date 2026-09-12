# Completion needs two independent classifications

**Origin:** the cross-build review and executable regressions on 2026-09-11.
The earlier tests passed while several public APIs and failure cases were wrong.

A handled application conflict normally means refused, and a 409 server span stays
unset. That does not imply every refused outcome has an unset span: if the actual
server status is 503, the span must still be an error. The regression combines
503 with a conflict kind to prevent outcome and span status from collapsing again.
Error type follows termination, selected timeout/canceled, exact 5xx text, then
handler_error. Rust's old generic "5xx" lost the actual server status.

Absence is also a separate question from error classification.
Rust's Option<&PublicInfo> answers whether a failure exists. public_info(None)
instead means that an existing failure is unknown. The old implementation treated
absence as an unknown error and always returned Some(problem). Cloning the public
projection owns its strings/maps; Option carries presence without a fabricated
all-zero value. HTTP re-exports telemetry::Outcome instead of defining a second enum.

The first-valid completion gate validates facts and monotonic duration before
consuming its terminal state. A rejected negative duration can be corrected; a
second valid finish produces no output. Each callback gets one attempt even when
another callback fails. Tests inspect these callbacks independently rather than
letting a fake observer silently deduplicate them.

&mut self makes one request-local gate exclusive. The native body owns the guard;
Drop closes otherwise abandoned observations. The callback vector uses boxed
FnMut trait objects because outputs have different closure types. catch_unwind
contains instrumentation panics; AssertUnwindSafe is limited to the output attempt,
not a claim that arbitrary application state is transactional.

**Used in:** src/shared/http/policy, problem and lifecycle; ordinary regression tests
were written with implementation visibility. The four selected mutations probe
these lessons; they do not establish an exhaustive mutation score.
