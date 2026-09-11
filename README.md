# n2f-rs

The Rust member of **nine to five**: independently usable backend blueprints with familiar ownership, composition, and documentation practices across stacks.

The starting point is a single library crate. It establishes module locations and architectural intent. Local PostgreSQL, Redis, Mailpit, S3 and observability are available through Compose. There is no executable server, web framework, persistence adapter, or business workflow yet.

The first leaf foundation is [shared/errors](src/shared/errors/mod.rs): shared
classification and public projection, owned failure data and typed context.
Its [contract](src/shared/errors/CONTRACT.md) names the implemented scenarios;
[integration tests](tests/errors_spec.rs) exercise the public Rust API.

The next foundations are [clock](src/shared/clock/README.md) and
[id](src/shared/id/README.md): system/manual clocks, validated UUID values,
controlled UUIDv7 generation and finite test sequences. The only direct crate
dependency is getrandom 0.4 for the ID module's OS entropy adapter. Cargo.lock pins
the resolution. `--offline` works after the locked dependencies have been cached.

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
shared instrumentation expectations. Application adapters are not connected yet.

## Find things

| Location | Responsibility |
| --- | --- |
| `src/lib.rs` | Library entry point and module declarations |
| `src/root/` | Future composition, process lifecycle, and cross-domain coordination |
| `src/domains/` | Future business modules with their own rules and adapters |
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
