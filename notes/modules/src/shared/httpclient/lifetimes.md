# One outbound attempt, with owned lifetime

A received 409 is a response value. A timeout is a transport failure and does not
establish whether the remote application committed. Retrying belongs with the
operation that understands idempotency, not this generic client.

Arc shares the client owner; an acquired permit bounds concurrent attempts and returns when dropped. tokio::select! races the owned request future against shutdown. Dropping the future cancels local polling, not a remote side effect. reqwest disables proxy inheritance, redirects and retries explicitly.

The origin is construction-time policy. Relative input cannot redirect to another
origin, and redirects are returned without following them. Environment proxies,
cookies and inbound authorization are not silently inherited. Bearer credentials
are disclosed only to the native request adapter. Response bytes are bounded even
when Content-Length is absent or misleading.

Explicit trace propagation is the only cross-request context carried by the leaf.
See [the concrete OTel note](otel/parent-and-completion.md) for SDK ownership.
Loopback tests cover statuses, redirects, oversized responses, deadline, cancellation,
origin escape, explicit propagation and close. They do not establish arbitrary
upstream retry safety or internet TLS deployment policy.
