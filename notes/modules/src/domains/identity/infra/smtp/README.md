# SMTP mail: the port stays configurable, and the failure words stay fixed

**Origin:** implementing M10–M16 spec-first on 2026-09-12 against an in-test fake
SMTP server, then running the process verifier against Mailpit.

lettre's `relay` and `starttls_relay` constructors are the obvious entry points,
but each one pins its own port (465 and 587) and derives the TLS parameters for
you. The adapter instead starts from `builder_dangerous(host)` and sets the port
and `Tls::None`, `Tls::Required` or `Tls::Wrapper` explicitly, so the configured
port always wins. The name is alarming but only means "you choose the TLS mode";
the `none` mode is still refused for a non-loopback host at construction, where
the TLS parameters are also built once so a bad host name fails at startup rather
than on the first mail.

The attempt is bounded twice. lettre's `timeout(Some(..))` applies per SMTP
command, which does not bound a slow sequence of individually fast replies, so an
outer `tokio::time::timeout` bounds the whole send, connect included. A stalled
server showed both paths: whichever fires first, the outcome word is `timeout`.
Every lettre error is reduced to one of `refused` (a 4xx or 5xx reply),
`timeout` or `failed` (connection, TLS or anything else) before it reaches the
log, so SMTP reply text such as a server's rejection message never leaves the
adapter. The spec's fake server returns a distinctive rejection text to prove it.

lettre picks the body's transfer encoding itself. A link line of roughly 85
characters exceeds the 78-character guideline, so the plain-text body may arrive
quoted-printable or base64 rather than 7bit, and `=` inside `#token=` may be
encoded. The spec decodes by the declared `Content-Transfer-Encoding` instead of
asserting the raw DATA, which keeps it honest about what a mail client sees.
Mailpit's `Text` field is already decoded, which is what the verifier reads.

The lettre `pool` feature is not enabled, so every send opens and closes its own
connection, matching M12's one connection per message without extra code.

Two root details are not visible in the adapter. The env reader records every
plain setting's value for the manifest, so SMTP host, username, From address and
link origin are shown by name only through a root-side redaction list rather than
by pretending they are secrets. The password is read only when a username is
configured: reading it unconditionally would make a missing password a required
error, and reading it as a plain string would record it. A stray password without
a username is therefore unused rather than refused; the adapter still enforces
both-or-neither for what it receives.

The refusal of mail to an inactive principal sits in the application, before the
already-verified check, not in the adapter: the credential read already carries
principal status, and the adapter must stay unaware of who may receive mail.

**Limits:** real TLS (`starttls` and `tls`) and SMTP authentication were only
constructed, never exercised against a server; the process verifier uses Mailpit
without TLS or credentials. No retry, bounce handling or durable mail queue exists
(A15).

**Used in:** src/domains/identity/infra/smtp/mod.rs, src/root/auth.rs and
tests/identity_smtp_spec.rs. See [the contract](../../../../../../../src/domains/identity/infra/smtp/CONTRACT.md).
