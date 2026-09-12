# HTTP native surface

The pure leaves implement [CONTRACT.md](CONTRACT.md) and [SHAPES.md](SHAPES.md).
They do not import SDK or framework types.

- problem_of(Option<&PublicInfo>) -> Option<Problem>: None is absence.
  public_info(None) separately projects a present unknown failure.
- Problem::json() owns lowercase wire members and omits absent optional values.
- classify_completion(CompletionFacts) -> Result<Classification, Failure>.
- Outcome is re-exported from telemetry.
- Active::begin(started, outputs); finish(facts, now) -> Result<bool, Failure>.
- axum::Server consumes its own Observer/Observation traits, an explicit scope
  factory and owned handler futures. current() reads the request task-local value.
- OTel future context is activated per poll, never by holding a thread-local guard
  across await.

The concrete otel observer maps one completion to duration metrics, a SERVER span
and a safe OTLP log. Scope/error details enter log attributes, never metric labels.
Root owns registered route names, provider resources, admission limits and shutdown.
