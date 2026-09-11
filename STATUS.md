# Status

## 2026-09-11 — Clock and ID leaf foundations

Implemented separate [src/shared/clock](src/shared/clock/README.md) and
[src/shared/id](src/shared/id/README.md) modules after local contracts, native module docs
and public specification tests. Initial red runs stopped on missing APIs/imports;
those were build/import failures, not executed assertion failures. The same author
wrote the specs and implementation with reference visibility.

Clock separates wall timestamps from monotonic elapsed readings. Its fake allows
wall correction without changing elapsed time and refuses invalid advances before
mutation. IDs parse standard-variant UUIDs independently of generation version;
V7 uses injected wall time and OS entropy, a guarded counter and failure-atomic
state. Rollback holds the timestamp; exhaustion returns a classified refusal until
wall time advances. Sequence owns finite fixtures. Timers and transport/storage
integration remain future work.

`cargo fmt --check`, `cargo test --offline --locked`, all-target clippy with
warnings denied, rustdoc with warnings denied and the time_and_ids example passed
on Rust/Cargo 1.86.0. The integration suites contain 26 tests (6 clock, 9 ID,
11 errors). The compiled example printed the expected two IDs and 25ms.
getrandom 0.4.1, cfg-if and libc compiled on this macOS host; other targets,
including higher-MSRV target-specific lockfile entries, remain unverified.

The isolated mutation runner caught 9/9 selected valid faults in this clone,
25/25 across the three builds. Each counted mutant compiled and failed a named
runtime expectation. Baselines before/after passed; source/test/config hashes
confirmed restored copies and unchanged originals. This is targeted evidence,
not an exhaustive mutation score or independent test authoring.

The entropy consumer exposed a real error-API gap: an already boxed Error cannot
be passed through the existing generic source builder. Added with_boxed_source,
E11 and a targeted mutation, retaining the original concrete source for downcast.
See [the finding](notes/modules/src/shared/errors/boxed-sources.md).

Added language walkthroughs, initial-red records and mutation evidence under
`notes/modules/src/shared/clock/` and `notes/modules/src/shared/id/`, preserving the 1:1
source-directory mapping. Decision 0004 records the state and ordering policy.
Local documentation links and module-note ownership were checked. Infrastructure,
logger/provenance integration, persistence uniqueness and Flover compatibility
are not established by these leaf checks.


## 2026-09-11 — Module notes mirror their source directories

Moved the error module's six notes/evidence files under
[notes/modules/src/shared/errors/](notes/modules/src/shared/errors/README.md), matching
the repository-relative source directory exactly. Removed the redundant
errors- filename prefix and added a local module index.

Updated the notes guide and AGENTS.md with the directory-mirroring convention,
including nested owners and source roots. Rebased incoming links and all relative
links inside moved notes. Historical status entries and accepted decision records
received link-target repairs; their decision prose was preserved.

Verification: every module-note directory maps to an existing source directory;
all old flat note paths are gone and no stale filename references remain. Local
Markdown links resolve within their own clone. Moved note text is unchanged apart
from link targets; mutation evidence is byte-identical. Production, tests and
configuration hashes are unchanged, so application suites were not rerun.


## 2026-09-11 — Language review notes for the error foundation

Added a [code walkthrough](notes/modules/src/shared/errors/language-walkthrough.md), three
language notes and two reusable testing-technique notes. Topics cover enum/pattern/Result syntax; ownership, consuming builders and borrowed lifetimes; traits, dynamic sources, Send/Sync and generic context.
Each explanation connects syntax to the ownership or behavior it supports,
includes examples and gotchas, and links to primary documentation where relevant.

The walkthrough provides the local source/test links; transferable language and
technique notes stay independent of repository-specific paths. Updated the notes
index and errors implementation guide to make the reading order discoverable.
The testing notes explain public-interface scenarios, type checks versus runtime
assertions, mutation evidence and why preserved metadata keys need explicit tests.

Verification: extracted the complete worked program from each language's
walkthrough, compiled and ran all three, and compared stdout with the documented
output. Go used the local module, Rust an offline temporary consumer crate, and
TypeScript the installed compiler with strict NodeNext settings followed by Node.
All three matched. Checked local Markdown links and clone independence; the
shared technique copies match. Existing source, test and configuration hashes
are unchanged.

This was documentation work. Full application suites, infrastructure checks and
mutation runs were not repeated. Short declaration/method excerpts are explained
as fragments; the complete walkthrough programs are the examples executed here.


## 2026-09-11 — Mutation checks on the current errors implementation

Ran selected mutations in isolated temporary copies of the current implementation
and tests. This build caught 3/3 valid faults; the three builds caught 10/10
in total. Every counted mutant compiled and failed a named assertion. There were
no survivors or invalid mutants in the completed runs. Initial and restored
baselines passed, and source/test/configuration hashes confirmed the working
files were unchanged.

The added E10 case alone catches replacing fields instead of merging them in all
three languages. No implementation or test changes were needed. See the
[mutation findings](notes/modules/src/shared/errors/mutations.md) and
[machine-readable results](notes/modules/src/shared/errors/mutations.json) for exact edits,
failing cases and limits.

These selected checks are not an exhaustive mutation score. No infrastructure or
transport integration was exercised.


## 2026-09-11 — Align the errors implementations

Separated vocabulary, classification, owned failure, typed context and public
projection into private modules with the existing public exports preserved.
E10 now checks metadata collisions, source replacement and empty Type/message
behavior through context. It passed the existing implementation before the
refactor; this was regression coverage, not another red-to-green cycle.

Ten public-API integration tests pass. Clippy checked all targets with warnings
denied; rustdoc passed with warnings denied; cargo fmt passed. The runnable
registration example produces the same four public values as Go and Nest.

Each clone carries a local [implementation guide](src/shared/errors/README.md) with the
cross-language API comparison and a matching registration-refusal example.
The [consistency note](notes/modules/src/shared/errors/consistency.md) explains why public
fallback and classification are separate questions.

The common contract text is aligned across the three builds. Local Markdown
links were checked. No new dependencies or transport integrations were added;
HTTP/WebSocket encoding and Flover compatibility remain unverified. Earlier
targeted mutation results belong to the first slice and were not rerun here.
Infrastructure was not started.


## 2026-09-11 — First errors implementation through spec tests

Implemented [the first errors contract](src/shared/errors/CONTRACT.md) with shared scenarios
E01-E09 and language-specific ownership/inspection behavior.

Rust uses Classified and a borrowed Classification view for public meaning.
Context<E> retains the typed case, while Failure owns metadata and a boxed
Error + Send + Sync source. No Clone bound is imposed on sources and no domain
registry is needed.

The initial run compiled and all nine integration tests failed against placeholder behavior.

Nine public-API integration tests pass. Clippy checked all targets with warnings denied; rustdoc passed with warnings denied. The standalone errors example is included. Two valid targeted mutations were detected after separate
compilation checks, and the restored baseline passed. Formatting and local
documentation links were checked. See the
[TDD note](notes/modules/src/shared/errors/first-slice.md) for findings and limitations.

No new runtime dependency, logger, provenance, retry engine, transport adapter, HTTP
problem envelope or WebSocket error mapping was added. No database, broker or
frontend integration was verified. Existing infrastructure services were not
started for this pure error-value slice.

## 2026-09-11 — Draft errors module contract

Added [src/shared/errors/mod.rs](src/shared/errors/mod.rs) as a documentation-only
module and declared it under `shared` so rustdoc includes the contract. It mirrors
the Go draft's ten kinds, stable Type identifiers, public disclosure rules,
annotation/translation distinction, aggregate handling and recovery ownership.
Rust-specific guidance retains typed Result errors, explicit classification,
source chains and owned metadata. The trait/mapping and context/source APIs remain
proposed; no error types, functions, dependencies or behavior tests were added.

Updated navigation and recorded the
[Rust adaptation finding](notes/modules/src/shared/errors/rust-adaptation.md). Each clone
carries its own complete draft; there is no sibling source or documentation
dependency.

Verification on Rust 1.86.0, with build artifacts in a temporary target directory:

- `cargo fmt --check` and `cargo check --offline` passed.
- `cargo doc --offline --no-deps` passed with rustdoc warnings treated as errors;
  the generated errors module page contains the contract sections.
- `cargo test --offline` passed with zero unit tests and zero documentation tests.
  This establishes scaffold compilation, not error behavior.
- The ten kind names, order and definitions match the Go and NestJS drafts.
  Local links in the changed Markdown documentation resolve.

No runtime error handling, transport integration or frontend compatibility was
implemented or verified in this documentation pass.

## 2026-09-11 — Observability, mail, objects and selected interfaces

Expanded default Compose startup with Mailpit 1.31.1, SeaweedFS 4.46 in single-node
mini mode, and Grafana OTEL LGTM 0.32.1. Each has project-scoped persistence and
loopback ports. The telemetry health check probes live collector and store endpoints.
Mailpit captures SMTP locally. SeaweedFS initializes local S3 credentials and a
bucket; unused WebDAV, admin UI and table catalog endpoints are disabled.

Added [OBSERVABILITY.md](OBSERVABILITY.md) with the shared instrumentation
expectations and a standalone, standard-library Python OTLP probe. Updated the
architecture and decision records to select standard WebSocket, SMTP and S3
boundaries and to distinguish transactional outbox recording from replaceable
broker delivery. No application SDK or language dependency was added.

Verification on Docker 27.4.0 / Compose v2.31.0-desktop.2:

- All three Compose files resolved with defaults and example environment overrides.
- All 15 containers ran healthy together using the intended separate ports.
- Each stack captured a local SMTP message and accepted signed S3 PUT/GET requests.
  The distinct object contents matched their own blueprint; anonymous GET returned 403.
- Each blueprint's synthetic probe exported OTLP HTTP logs, metrics and traces.
  Authenticated Grafana data-source queries retrieved the trace, the log matching
  its trace ID, and the expected metric series from the respective stores.
- The first metric query failed because Prometheus normalized the name and appended
  a unit suffix. Inspecting the actual series resolved this; the probe output,
  guide and substrate note now use `n2f_infra_smoke_ratio`.
- Mail, object contents and all three telemetry signals remained retrievable after
  forced recreation of the three new services in every stack. This also verified
  the final SeaweedFS options with the table catalog endpoints disabled.
- Shell syntax, Python syntax/CLI help and documentation links were checked.
- Temporary `n2f-*-infra-check-20260911` containers, networks and all their volumes
  were removed. The regular development stacks are not left running.

The local integration harness was created with implementation visibility and is
not an independent or blind test. Docker/network checks required approved access
outside the sandbox. Language application suites were not repeated because their
code and dependencies were unchanged.

Not implemented or verified: application OTel SDKs, WebSocket endpoints, outbox or
JetStream delivery, infrastructure exporters, application dashboards/alerts,
profiling, collector outage behavior under application load, OTLP gRPC export,
S3 presigning/browser CORS, full S3 feature parity, crash recovery or deployment
hardening. Synthetic acceptance and retrieval demonstrate local infrastructure,
not application instrumentation or Flover integration.

## 2026-09-11 — Local PostgreSQL and Redis

Added independent `compose.yaml` and `.env.example` files with PostgreSQL 18.6
and Redis 8.10.1, loopback ports, readiness checks, project-scoped volumes,
Redis AOF, and bounded Docker logs. The equivalent setup in each n2f repository
uses its own default project name and host ports.

[INFRASTRUCTURE.md](INFRASTRUCTURE.md) records operation, reset behavior and
proposed next integrations. Added the local-infrastructure ownership decision and
a substrate note about PostgreSQL 18's versioned Docker data layout. The current
README and architecture distinguish running dependencies from implemented adapters.

Verification on Docker 27.4.0 and Compose v2.31.0-desktop.2:

- All three files resolved with defaults and with their `.env.example` files.
- All six services ran together and became healthy on their distinct loopback ports.
- PostgreSQL reported 18.6 and `/var/lib/postgresql/18/docker`; Redis reported 8.10.1.
- Authenticated PostgreSQL TCP writes and Redis writes succeeded. Each build stored
  a distinct marker, and every marker survived forced recreation of its containers.
- Inspection confirmed the PostgreSQL named volume covers the versioned data path.
- Disposable `n2f-*-verify-20260910-c1a7` projects, containers, networks and volumes
  were removed after checking. The normal development stacks were not left running.

The initial Docker socket check and one database check were refused by the sandbox;
rerunning with approved Docker access succeeded. Application code and dependencies
were unchanged, so language build/test suites were not repeated. No application
integration, crash recovery, database upgrade, backup/restore, events, socket
transport or telemetry pipeline was tested or implemented in this pass.

## 2026-09-10 — Initial scaffold

The n2f family starts with independent Go, NestJS and Rust directories. This
pass establishes module ownership and the notes/decision workflow. Current
runtime behavior and commands are documented in [README.md](README.md).

No product domain, shared logger, provenance module, persistence or authentication
has been added. Architecture rules and decision immutability are held by review.

Added a dependency-free library crate with `root`, `domains` and `shared` module
locations. There is no executable server. HTTP framework, async runtime and
persistence choices remain open.

Verification on Rust and Cargo 1.86.0:

- `cargo fmt --check` passed.
- `cargo check --offline` passed.
- `cargo test --offline` passed with zero unit tests and zero documentation tests.

These commands establish that the crate scaffold builds. They do not establish
application behavior, architecture enforcement or backend compatibility.
