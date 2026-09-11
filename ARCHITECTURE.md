# Architecture

## Starting point

n2f-rs starts as one library crate with three module locations: `root`, `domains`, and `shared`. The intended application shape is a modular monolith, following the ownership and explicit composition used in Overwatch. Its eventual process entry point, HTTP framework, async runtime, storage adapters, and first business domain remain undecided. PostgreSQL 18 and Redis are available through local Compose.

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

Rust modules can begin as files and become directories when they grow. Commands and queries distinguish responsibilities; they do not require separate databases, generic handlers, or an event bus. Migrations and SQL belong with their owning persistence adapter when the first SQL adapter is implemented.

Keep the module's exported surface small as behavior develops. The public empty modules in the current scaffold are navigation points, not settled application APIs.

## Behavior before machinery

Define each operation's observable behavior before selecting its machinery: validation failures, authorization outcomes, consistency, conflicts, and the meaning of success or an uncertain result. Preserve distinctions callers need, including Flover frontends. Comparable scenarios across n2f stacks should exercise the same promises through idiomatic implementations.

Transaction boundaries and event delivery must be explicit when writes arrive. Neither atomicity nor durable publication follows from folder layout. Persistence, migrations, outbox delivery, authentication, configuration, logging, and provenance are not implemented or specified by this scaffold.

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
The first event and WebSocket implementations still need concrete wire contracts.
