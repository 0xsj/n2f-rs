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
- Feature routes (H15–H18): Route { path, method, operation, admission, handler };
  the handler's RequestContext carries `request: Arc<Request>` (method, template,
  query, bounded body bytes, selected Headers as lists, cookies as name → values,
  trusted source key, admitted value) and returns Result<Response, RequestFailure>.
  Response::json / Response::empty with with_status, with_header (allowlisted) and
  with_cookie (Cookie, SameSite; serialize() enforces the `__Host-` rules; a
  Max-Age of 0 serializes as `Max-Age=0`, the clearing form).
  validate() runs before commitment; a violation is a handler error.
- Refusal { failure, headers, cookies } is the headers-carrying refusal: a handler
  returns Err(RequestFailure::Refused(refusal)) and admission returns
  Admission::Refused { failure, headers, cookies }. The failure alone selects the
  status and problem document (H01/H02); the headers obey the Response allowlist
  and the cookies serialize like a response's, so a 401 can clear a session
  cookie and a 429 can carry Retry-After. validate() failures are handler errors.
- Admission: Admit = Fn(Arc<Request>) -> Admission { Anonymous | Authenticated
  { initiator, tenant, admitted } | Refused { failure, headers, cookies } }, called after
  routing and before Open, at most once, never for 404/405. Open now receives
  Option<Attribution>; None means the root's anonymous default. Config.source
  derives the trusted source key from the Peer extension and headers.
- json_work(future) adapts a JSON-only handler to the feature work type;
  parse_cookies and allow_set are pure helpers with their own tests.
- OTel future context is activated per poll, never by holding a thread-local guard
  across await.

The concrete otel observer maps one completion to duration metrics, a SERVER span
and a safe OTLP log. Scope/error details enter log attributes, never metric labels.
Root owns registered route names, provider resources, admission limits and shutdown.
