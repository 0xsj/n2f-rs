# Feature routes: admission before the scope, values instead of the framework

**Origin:** implementing H15–H19 on 2026-09-12 against the axum adapter, spec
first with a real hyper server on an ephemeral port.

The contract's decisive constraint is provenance: attribution is fixed when a
scope is entered and cannot be changed later. That forced the request pipeline
into routing, body, cookies, admission, open, handler. Before this change the
scope was opened first; now every failure that precedes admission (404, 405,
415, 413, 408, a malformed Cookie header) still opens an anonymous scope so the
problem document keeps its request and correlation IDs, and a refused admission
opens its scope under the fixed `http.admission` operation before the refusal is
projected. The spec proves the order with one shared call log across the
admission function and the open callback, not two lists compared by hand.

The framework request cannot be held across an await unless its body type is
Sync, which hyper's is not. Everything the handler may need (selected header
lists, the cookie headers, the peer address and the derived source key) is
copied into owned values inside one block before the body is read; the closure
that reads headers dies with that block. The `Request`/`Response` names collide
with hyper's, so the framework types are imported under `HttpRequest` and
`HttpResponse`.

The admitted value travels inside the request value, as the contract asks, which
means the request is built twice: once without it for the admission function and
once with it for the handler. `Request` is Clone for that reason; the body is
copied once more for authenticated requests, bounded by the same limit.

Cookies are parsed strictly. RFC 6265 permits a bare name without `=`, but an
`=`-less pair in a Cookie header is treated as malformed here, along with quoted
values, spaces inside a value and control characters. Duplicates stay visible as
a list so the identity transport can refuse a duplicated session cookie itself.

The response value is validated before commitment, and a violation becomes a
handler error with a 500, never a silent drop. The old `json!({"ok":true})`
default body is gone: a handler that returns no body now sends none, and
`content-type` is only set when there is one. `json_work` keeps the diagnostic
routes one-line.

A refusal is a value, not just an error. The first cut cleared every header
and cookie on any failure, which made three transport rules unreachable: a 401
that clears the session cookie, a 429 that carries Retry-After, and no-store on
problem responses. `Refusal { failure, headers, cookies }` (as
`RequestFailure::Refused` from handlers and the `cookies` member of
`Admission::Refused`) keeps the problem projection driven by the failure alone
while the owner's allowlisted headers and Set-Cookie values are written. Its
`validate` reuses `Response::validate`, so the allowlist is enforced once and an
unlisted header on a refusal is a 500 like it is on a response. The adapter's
own `RequestFailure::classification` accessor replaced three hand-written
matches, which is what let the new variant land without touching the part 1
mutation sites.

**Limits:** the admission function receives the request value only; the trace
parent is not handed to it separately (the observation instruments handler work,
not admission). Timeouts are shared: admission runs under the same server
timeout as the handler. reqwest in this crate has no `json` feature, so specs
decode text. tools/verify_http.py was not rerun for this change.

**Used in:** src/shared/http/axum/mod.rs, src/root/http.rs and
tests/http_feature_spec.rs. See [design notes](design-and-language.md).
