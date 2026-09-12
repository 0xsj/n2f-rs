# Export acceptance is not stored delivery

**Origin:** SDK installation, source/API inspection and real OTLP process checks
on 2026-09-11. Dependencies are pinned in the local manifest and lockfile.

Rust uses OTel 0.31.0, Axum 0.8.6 and Tokio 1.47.1 on Rust 1.86. The blocking reqwest exporter uses rustls WebPKI roots for HTTPS. Compatible transitive resolution is locked.

Trace/log queues are bounded SDK queues; metric aggregation has finite dimensions
for this diagnostic registry. Request completion never waits for network export.
The root validates queue/batch bounds, timeout, interval, endpoint, resource and
sampling before listening. The no-op branch constructs no providers.

Generic SDK diagnostic events are not exact dropped-record counts. Outage tests
observed safe diagnostics while all HTTP scenarios still passed. A failed export
is not proof that no backend accepted any data. Retries and shutdown are owned by
the SDK/root; there is no unbounded second application queue.

Root stops HTTP admission and drains admitted work before provider/logger shutdown,
using one remaining budget. Repeated provider close is stable. Rust caches a small copyable close classification, rather than cloning Failure, which can own a non-cloneable source error.

The retrieval tool asks Tempo for a process trace, Loki for linked completion logs,
and Prometheus for the matching service instance's histogram. It uses event times
instead of an arbitrary last-five-minutes window. The metric's stored name has the
seconds suffix. Sampling-out runs retrieved logs and metrics with non-recording
trace identities.

The recovery fixture initially forwarded an empty Node export: Python's test proxy
read Content-Length while Node sent chunked transfer encoding. The proxy now decodes
chunks and preserves Content-Encoding. The stored-data check caught what an HTTP
200 acceptance check would have missed. After recovery, all three produced stored
telemetry within the same process; pre-recovery lost logs are not claimed recovered.

**Used in:** src/shared/telemetry/otel/ and tools/telemetry/verify_http_delivery.py.

Primary references: [Go exporters](https://opentelemetry.io/docs/languages/go/exporters/),
[Rust SDK](https://docs.rs/opentelemetry_sdk/0.31.0/opentelemetry_sdk/),
[JavaScript exporters](https://opentelemetry.io/docs/languages/js/exporters/).
