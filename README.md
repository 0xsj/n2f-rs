# n2f-rs

The Rust member of **nine to five**: independently usable backend blueprints with familiar ownership, composition, and documentation practices across stacks.

## Current state

Errors, clock, UUIDs, secrets, env parsing, provenance and logging are implemented
with local contracts and executable tests. The composition root owns typed config,
service identity and bounded output delivery. The [foundations command](FOUNDATIONS.md)
exercises them together through console, JSON and no-op adapters.

The logger uses tracing 0.1.44, tracing-subscriber 0.3.23 and serde_json 1.0.149
inside its adapter; UUID entropy uses getrandom 0.4.1.
Validation/pagination, PostgreSQL transactions and migrations, readiness, outbound
HTTP, WebSockets and a transactional outbox with a replaceable publisher are now
implemented. [JetStream](JETSTREAM.md) is also implemented and verified as a second
publisher, with a durable handoff into the PostgreSQL mailbox. See [the infrastructure guide](INFRASTRUCTURE_BUILD.md) for contracts,
commands and limits. [Identity principal leaves](src/domains/identity/domain/CONTRACT.md) now implement
registration values, restoration and versioned suspend/activate transitions.
[Domain ownership and order](DOMAINS.md) describe the next application/persistence,
audit and org slices. [Built-in authentication](AUTHENTICATION.md) is now specified
as required baseline scope, with credential/session/challenge contracts, a logical
schema and native API map. No persisted identity workflow or login is implemented yet.

The [telemetry into HTTP slice](TELEMETRY_HTTP.md) now runs a diagnostic server:
provenance admission, safe problem responses, isolated request context, completion
logs and OTLP traces/metrics/logs. The guide includes settings and verification.


## Work locally

The [errors implementation guide](src/shared/errors/README.md) maps the APIs across Go,
Rust and Nest, explains intentional differences, and points to the local example.

From this directory, using a Rust toolchain that supports edition 2024:

```sh
cargo fmt --check
cargo check --locked
cargo test --locked
cargo doc --locked --no-deps
cargo run --locked --example errors
cargo run --locked --example time_and_ids
```

The scaffold and shared foundations were checked with Rust and Cargo 1.86.0.
The error contract has behavior tests and a [runnable example](examples/errors.rs).
There is no business-workflow integration suite yet.

## Local infrastructure

PostgreSQL 18, Redis, Mailpit, S3-compatible storage and observability run through
this repository's [compose.yaml](compose.yaml):

```sh
docker compose up -d --wait
```

See [INFRASTRUCTURE.md](INFRASTRUCTURE.md) for ports, optional environment overrides,
data lifecycle, and selected events/WebSocket direction.
[OBSERVABILITY.md](OBSERVABILITY.md) contains the synthetic telemetry example and
shared instrumentation expectations. The diagnostic HTTP adapter exports all three
signals; see [its run guide](TELEMETRY_HTTP.md).

## Find things

| Location | Responsibility |
| --- | --- |
| `src/lib.rs` | Library entry point and module declarations |
| `src/root/` | Foundations composition and process lifecycle; future cross-domain coordination |
| `src/domains/` | Business modules, starting with identity principal leaves |
| `src/shared/` | Shared foundations with a concrete consumer and named responsibility |
| `notes/` | Discoveries, experiments, and transferable learning |
| `decisions/` | Consequential choices and their alternatives |

Read [architecture](ARCHITECTURE.md) for the dependency rules and future module shape, [working guidance](AGENTS.md) for repository conventions, and [status](STATUS.md) for implemented work and remaining gaps. The [notes index](notes/README.md) and [decision index](decisions/README.md) keep learning and commitments discoverable.

## Targeted foundation mutations

After installing the normal toolchain/dependencies, run:

```sh
python3 tools/mutations/foundations.py
```

The runner builds and tests isolated temporary copies, retaining logs and hashes.
Its selected mutations probe clock/ID contracts; they are not an exhaustive score.
See the [notes index](notes/README.md) for language walkthroughs and recorded evidence.

## Process verification

`python3 tools/verify_foundations.py` builds the real executable and verifies eleven
process scenarios, including terminal color and safe invalid-config exit.
`python3 tools/mutations/process.py` probes selected secret/env/provenance/logger/root
faults in isolated copies. These are targeted checks, not exhaustive guarantees.
