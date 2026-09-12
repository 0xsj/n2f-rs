# One completion feeds three signals

**Origin:** native process exports retrieved from Tempo, Loki and Prometheus on
2026-09-11, including sampling-out and collector recovery.

The HTTP adapter supplies normalized method, a registered template, final facts
and one monotonic duration. The OTel observer uses the same facts for the SERVER
span and http.server.request.duration in seconds. Unmatched routes omit http.route;
raw path/query/header values are never fallback dimensions.

The completion log links the active trace/span and carries safe n2f scope/work/
correlation identifiers plus the public error projection. These identifiers and
messages are log attributes, never metric dimensions. The local logger independently
binds its protected scope/error/trace envelopes.

Sampling all/none belongs to root. A valid non-recording span still yields trace
identity; metrics and completion logs remain enabled. The real verifier sends a
remote unsampled parent and checks that local sampling policy is applied, while
valid trace identity continues with a fresh span ID.

The Rust observer keeps Context in the concrete adapter and wraps the handler
future per poll. SDK providers use dedicated batch/export threads. The root creates
and shuts down the blocking HTTP exporters outside Tokio request tasks. The explicit
log trace context preserves linkage even when no span is recording.

**Used in:** src/shared/http/otel/, src/root/http and the stored-delivery verifier.
