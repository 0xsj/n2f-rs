# Telemetry native surface

Trace identity and Outcome are implemented SDK-free value leaves.
TraceRef::parse returns Result<TraceRef, Failure>; snapshot returns owned strings.
Outcome is a closed enum with parse/as_str. logger.with_trace derives a child logger.

The concrete otel directory implements validated provider configuration and bounded
SDK batch delivery for traces/logs, periodic metrics and repeat-safe shutdown.
Root supplies service resources and a remaining deadline. None mode constructs no
providers. SDK failures do not become HTTP/business failures.

HTTP owns the consumer-specific observation capability. SDK Span, Context, provider
and exporter types do not enter these value leaves or application contracts.
See the root [run/settings guide](../../../TELEMETRY_HTTP.md).
