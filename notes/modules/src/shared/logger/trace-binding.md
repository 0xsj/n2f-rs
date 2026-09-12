# Trace binding is an owned envelope

**Origin:** logger regression tests and HTTP exports on 2026-09-11.

Trace identity is projected explicitly into trace_id, span_id and sampled.
Ordinary fields named trace cannot replace the protected envelope, and deriving a
traced child does not modify its parent. The regression writes both records and
inspects their actual JSON. A non-recording trace keeps sampled=false.

with_trace clones the cheap logger handle and owns a serde_json projection. Rust's private TraceRef fields prevent callers constructing invalid values. Both the formatter and emitted JSON record include the trace envelope.

Local logger delivery and OTLP log export are separate adapters with separate
failure accounting. Neither is durable audit. Shared logger code imports telemetry
values, not its SDK delivery submodule.
