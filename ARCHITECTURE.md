# Architecture

## First domain slice

[Identity, audit and org ownership](DOMAINS.md) is defined. Identity implements
authentication, persistence and transport; audit ingestion and org value leaves
are also implemented. The org application port, transactional persistence and
domain transport follow those leaves. Principal references alone confer no
authentication or org authority.
[Authentication](AUTHENTICATION.md) is required within identity, independent of an
account/profile domain. Credential/session/challenge contracts precede their adapters.

## Starting point

n2f-rs starts as one library crate with three module locations: `root`, `domains`, and `shared`. The intended application shape is a modular monolith, following the ownership and explicit composition used in Overwatch. The foundations process now composes the implemented shared modules. Axum/Hyper on Tokio, SQLx PostgreSQL, reqwest and native WebSockets are selected and implemented. Identity and audit are integrated slices; org value leaves now establish the next business boundary. PostgreSQL 18 and Redis are available through local Compose.

Keep the foundation small enough that an ordinary feature has an obvious path. Add a boundary when it protects real behavior or isolates a real dependency. A placeholder folder does not establish a working capability.

## Ownership and dependencies

- **Domain code** owns business values, invariants, and lifecycle rules. It uses Rust types to represent meaningful states and stays independent of application operations, transport, storage, and runtime crates.
- **Application code** owns a use case, its authorization decisions, and required effects. It depends on its own domain and declares small consumer-owned traits when it needs an external capability. Framework, runtime, and storage types stay out of these contracts.
- **Infrastructure adapters** implement those traits and translate external representations into application or domain values. Database records and provider responses stay within their adapters.
- **Transports** decode requests, invoke application operations, and translate outcomes into caller-facing responses. They do not implement business rules or bypass the application to access storage.
- **The composition root** constructs concrete dependencies, wires operations and transports, and owns process startup and shutdown when a process exists. No other layer imports `root`.
- **Shared code** has a concrete consumer and named responsibility, and depends on neither business modules nor `root`. Its consumer may be the process or composition root before any business domain exists; multiple consumers are not a prerequisite. Keep business-specific behavior with its domain owner. If shared infrastructure becomes necessary, give it a distinct infrastructure module that owns its external imports; framework-independent shared types must remain independent of it.

Business modules do not import peer modules. A workflow involving several domains is composed at the root using their application entry points. Introduce events only when their delivery and consistency semantics serve a concrete workflow.

Keep dependency injection explicit through construction. Choose concrete types, generics, or trait objects to fit the actual consumer and ownership needs. Do not introduce a universal container or require a trait for every struct.

## Growing a business module

When the first workflow establishes a domain, use this shape as a guide. Only create the parts that the workflow uses; `<name>` is a placeholder, not an existing domain.

```text
src/
  domains/
    <name>/
      mod.rs
      domain/          values, invariants, lifecycle rules
      app/
        command/       changes and their required effects
        query/         reads and projections
      infra/           concrete persistence or provider adapters
      transport/       module-owned delivery adapters
  root/                concrete wiring and composed workflows
  shared/              foundations with a concrete consumer and named responsibility
```

Rust modules can begin as files and become directories when they grow. Commands and queries distinguish responsibilities; they do not require separate databases, generic handlers, or an event bus. Migrations and SQL belong with their owning persistence adapter; the event schema is owned by shared/events/postgres.

Keep the module's exported surface small as behavior develops. The public empty modules in the current scaffold are navigation points, not settled application APIs.

## Behavior before machinery

Define each operation's observable behavior before selecting its machinery: validation failures, authorization outcomes, consistency, conflicts, and the meaning of success or an uncertain result. Preserve distinctions callers need, including Flover frontends. Comparable scenarios across n2f stacks should exercise the same promises through idiomatic implementations.

Transaction boundaries and event delivery must be explicit when writes arrive. Neither atomicity nor durable publication follows from folder layout. Persistence, migrations, outbox delivery and authentication remain unimplemented. Configuration, logging and provenance now implement their local contracts and are composed by the [foundations command](FOUNDATIONS.md).

## What enforces this today

Code review currently enforces the dependency directions above. A single crate does not prevent sibling modules from importing each other, and no automated architecture check exists yet. Rust privacy can constrain public surfaces as real types are added, but it does not by itself prove these architectural rules.

Start with one crate to keep navigation and builds straightforward. Splitting modules into crates later can strengthen dependency boundaries, but requires explicit public APIs, ownership and error contracts, feature management, and build configuration. Extracting a module into a service would additionally require transport, consistency, and operational decisions. The current structure makes ownership visible; it does not promise cost-free extraction.

Record findings in [notes](notes/README.md), consequential commitments in [decisions](decisions/README.md), and implemented capabilities and verification limits in [status](STATUS.md).

## Local dependency topology

[Compose](compose.yaml) owns this clone's PostgreSQL, Redis, Mailpit, S3 and
observability containers, network and data volumes. [INFRASTRUCTURE.md](INFRASTRUCTURE.md) describes local operation.
Applications run separately for now. Adding a container does not select a driver,
create a migration, or supply an application port implementation.

## Selected foundation boundaries

[Observability](OBSERVABILITY.md) is a first-class process responsibility, with OTLP
as the export boundary and shared meaning across logs, traces and metrics. WebSocket
is the chosen realtime transport. SMTP and S3 clients live in owned infrastructure
adapters. Their SDK types must not become domain APIs.

For state-changing event workflows, atomically record the event in an outbox. Keep
delivery separately owned so JetStream or a simpler dispatcher can be supplied
without moving the transaction boundary. Swapping adapters requires equivalent
failure/retry/duplicate scenarios; an interface alone does not establish parity.
The event and WebSocket contracts and concrete diagnostics are implemented;
see [the infrastructure guide](INFRASTRUCTURE_BUILD.md).

## Provenance ownership

[src/shared/provenance](src/shared/provenance/README.md) implements immutable execution
scopes and stable WorkContext. Initiator, executor, represented principal and
tenant have explicit lifetimes. Replays open a new chain linked to original work;
extra causal links belong beside an owning envelope. Public correlation hints
and validated persisted work are different admission paths.

The core consumes shared IDs/errors and narrow clock/generation capabilities.
The logger's explicit scope projection is implemented. Runtime context carriers,
HTTP/WebSocket propagation, broker codecs, tracing and public/audit projections
remain adapters. Scope is not an authorization context, transaction receipt,
deduplication record or domain evidence model. P01–P19/P23 have core evidence;
P20–P22/P24 remain adapter/integration requirements.

## Telemetry into HTTP

[The diagnostic HTTP slice](TELEMETRY_HTTP.md) is implemented and verified with real
requests and stored OTLP signals.
Telemetry owns trace/outcome values and concrete SDK delivery. HTTP owns safe wire
projection, ingress admission and request completion, and declares the observation
capability its adapter needs. Root wires providers and shares service resources
and the shutdown budget. SDK types stay within root and concrete adapters; shared
value leaves and application contracts do not import them.

Use a real diagnostic request before introducing a business domain. Keep failures,
refusals, sampling and transport termination distinct. Neither an SDK dependency
nor an interface demonstrates export, delivery or frontend compatibility.

## Implemented request boundary

The diagnostic HTTP process composes existing foundations with the native adapter
and concrete OTel providers. One validated service instance is shared by logging
and SDK resources. Completion classification preserves application failure meaning
separately from the server's final response/termination facts.

Root stops admission, drains admitted work, then closes providers and local logging
with the remaining process budget. Collector failure has no response-policy branch.
Exact queue drops are not inferred from generic SDK diagnostics. The first consumer
is bounded JSON. PostgreSQL, outbound HTTP, sockets and events now have separate
owners and concrete diagnostics. Authentication and streaming business workflows
remain domain work.

## Infrastructure ownership before domains

The [implemented slice](INFRASTRUCTURE_BUILD.md) adds pure validation/pagination,
PostgreSQL, readiness, outbound HTTP, sockets and events in that order. Root owns
resource construction, readiness dependencies, diagnostic consumers and shutdown.
Database transactions expose driver types only to concrete persistence adapters;
domain/application ports must use their own vocabulary. No generic repository or
shared business transaction interface has been imposed before a domain needs one.

An outbox is an atomic recording mechanism. Publisher is a separate durable-delivery
capability. PostgreSQL mailbox and JetStream are implemented publisher adapters;
[real JetStream checks](JETSTREAM.md) cover publication and durable handoff. Mailbox deduplication and consumer effects
share a transaction. This single destination does not establish fanout, global
ordering, exactly-once processing or a future audit retention policy.
