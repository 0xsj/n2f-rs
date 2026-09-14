# HTTP transport: what the boundary can and cannot carry for a refusal

**Origin:** implementing R01–R15 spec-first on 2026-09-12 on the axum feature-route
boundary, with the stage 4 operations on an in-memory store, the real hasher with
a cheap work function and the real codec with counter entropy.

Attribution is fixed when the scope opens, so every session check runs in the
admission step, before the handler exists. The transport resolves the presented
cookie through the Authenticate query there, which also performs the idle
extension write; an anonymous POST that carries a stale session cookie tolerates
the rejection and binds its CSRF context to `anonymous`, while a session-required
or session-optional route turns the same rejection into 401. A duplicated session
cookie is refused before any lookup on every route, which the spec proves by
counting resolver calls.

The CSRF token is the whole cookie value, `nonce.issued.tag`, returned in the body
of GET /csrf and of a successful login. The binding is the hex of the session
digest or the word `anonymous`, so a context issued before login cannot be replayed
after it and a session-bound context cannot be replayed on the registration route
once the session is gone. The lifetime check is a separate arm of the same
condition, and the mutation that drops it is caught only by the expired-context
scenario; the forged-tag scenario would not notice.

Two things the shared boundary cannot express in this build. A handler failure is
projected without headers and without Set-Cookie, so R07's "401 with the session
cookie cleared" and R08's Retry-After on a rate-limit refusal are not reachable
from the transport: the 429 carries the right kind and code, but the header is
absent, and `tools/verify_auth_http.py` stops there. Both need a headers-carrying
handler failure in the shared http adapter, which is not this transport's to add.
Problem responses also lack `Cache-Control: no-store` for the same reason.

Duplicate JSON members are refused by a map visitor before serde builds the typed
body; `deny_unknown_fields` alone would silently keep the last duplicate. An empty
body on a route that expects one is a 400 from the same path, and a bodiless route
refuses any body rather than ignoring it.

Login refusals are re-projected into one fixed failure before they reach the
boundary; the transport is where a field naming the failing credential would be
added, so the spec asserts the problem document has no `fields` member.

**Limits:** cookies are asserted by serialized attributes, not by a browser; no
real hashing cost, storage or mail; the Secure attribute is exercised only through
serialization on the loopback listener.

**Used in:** src/domains/identity/transport/http/mod.rs and
tests/identity_transport_spec.rs. See [the local adapters](../../infra/local/README.md)
and [the root composition](../../../../root/auth-composition.md).
