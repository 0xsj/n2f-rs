# Root: composing authentication into the diagnostic process

**Origin:** AUTH_BUILD.md, 2026-09-12.

The env reader refuses a bound above its safe-integer ceiling as an invalid
definition, so a setting bounded by "any large number" refuses startup before the
value is read. The time settings use the common domain ceiling instead. The first
verifier run failed on exactly this, reported only as "configuration refused"; the
process prints no field for a refused definition, so the diagnosis needed a scratch
call to the loader with the same environment.

One clonable `AuthPorts` implements every application port by delegating to Arcs
of the concrete adapters. The hasher's `Outcome` becomes the port's `bool` and the
codec's `Issued` becomes the port's pair at this boundary, so the application never
names an adapter type. A second UUIDv7 generator serves identity IDs; the
provenance factory already owns the first, and two generators only mean two
independent counters.

The manifest is an `http.start` log record with the AUTH_* variables as the reader
recorded them (secrets already redacted to `[REDACTED]`), the blocklist entry
count and `auth.mail=disabled`. The base configuration still requires DEMO_TOKEN,
which the auth verifier had to supply like every other verifier.

**Limits:** the shared logger nests record fields under `fields`, so a verifier
reading `route` at the top level sees nothing; the verifier now looks in both
places. Retry-After on 429 is not emitted (see the transport note). Nothing here
is exercised in OTLP mode.

**Used in:** src/root/auth.rs, src/root/http.rs and tools/verify_auth_http.py.
