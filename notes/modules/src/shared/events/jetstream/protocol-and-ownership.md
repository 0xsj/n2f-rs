# What the real replacement test taught us

Rust uses impl Publisher for Broker with a lifetime-bound BoxFuture. The dispatcher still receives &dyn Publisher; no broker type enters its API. tokio::time::timeout bounds the owned operation, and dropping the future ends local polling without claiming the remote write rolled back. AtomicBool marks close immediately before draining the official async-nats 0.39.0 connection. That pinned version is compiled with the blueprint's Rust 1.86 toolchain.

## A request multiplexer is not always a pull subscription

The initial Go integration passed while Rust and Node timed out during mailbox
transfer. JetStream pull deliveries retain the original event subject. Those SDKs'
general request multiplexers identify replies by inbox subject, so a real delivery
could miss their pending-request lookup. Rust now supplies Request::inbox with a
fresh inbox; Node uses noMux: true. Each owns a dedicated subscription with a finite
request timeout. Real redelivery and confirmed acknowledgement tests pass after
that change. A fake Publisher could never have exposed this protocol mismatch.

## Receipt, duplicate and uncertain completion

A PubAck must name the configured stream and a positive stored sequence. The event
UUID is sent as Nats-Msg-Id. A duplicate PubAck does not compare payloads for us, so
the adapter reads the stored sequence and rejects conflicting content. The two-minute
window is a broker optimization; durable mailbox deduplication protects consumption
after that window. Immutable IDs and stable producer encoding remain requirements.

The test claims an outbox lease, publishes successfully, deliberately leaves the
lease unacknowledged, expires it and dispatches again. The broker retains one copy.
Transfer into a nondurable sink must fail and leave broker delivery unacknowledged;
after ack wait, transfer into the real mailbox succeeds. Mailbox effects commit once.
A closed publisher leaves a newly claimed outbox event pending instead of sent.

## Byte budgets include protocol overhead

A valid 65536-byte event still needs NATS headers. A stream max_msg_size equal to the
envelope limit can reject a valid envelope. Keep the envelope limit at 64 KiB and
allow 128 KiB broker messages, including headers. The real test publishes and consumes
an envelope exactly at the leaf limit. Its first fixture accidentally reused the
original event UUID; all three implementations correctly refused the conflicting
payload. The corrected fixture uses a distinct ID.

## Explicit provisioning and ownership

Root calls provision before starting its finite diagnostic. The adapter compares
selected required stream and consumer policies and refuses incompatible resources.
It never automatically updates a preexisting stream. The verifier deliberately
changes a stream's message limit, observes a safe root failure, and confirms the
operator's setting was not rewritten.

The implementation uses the documented JetStream JSON API over official NATS
connections. This small surface avoids SDK types escaping, but response validation
and subscription ownership remain adapter responsibilities. It is not a custom NATS
wire client; the tiny raw fixture client exists only inside the test verifier.

Retained fixture volumes also retain stream capacity reservations. Repeated tests
with fresh 64 MiB streams eventually exhausted the fixed 256 MiB server budget.
The runner now uses a fresh project per run, retains that run's volume, and stops
its own containers. Broker resource/API failures map to unavailable; an existing
resource with incompatible settings remains a separate invalid-configuration result.

Primary references: [JetStream API](https://docs.nats.io/reference/reference-protocols/nats_api_reference),
[message headers](https://docs.nats.io/nats-concepts/jetstream/headers),
[JetStream development](https://docs.nats.io/using-nats/developer/develop_jetstream).
