# A transaction needs an acknowledgement boundary

Origin: PostgreSQL 18 integration tests, including a callback that swallows SELECT
1/0, a failed multi-statement migration, concurrent writes and a delayed query.

SQLx 0.8.6 commit discards the PostgreSQL command tag. A SELECT 1 before commit detects an aborted transaction so swallowed statement failure cannot become success. Transaction Drop initiates rollback. The for<'c> callback borrows a connection for its own future lifetime; BoxFuture expresses that borrowed asynchronous work. A raw_sql future hit an Executor/Send lifetime error in the boxed callback; PgConnection's Executor::execute on the script compiles and executes the same trusted script.

A network error after COMMIT begins is database.commit_uncertain. It is not proof
of rollback and must not automatically trigger a retry. SQL uniqueness maps to
generic database conflict; the domain adapter decides which business condition it
represents. Driver text is deliberately excluded from generic failures because it
can contain SQL and credentials.

Migrations hold a transaction-scoped advisory lock, compare SHA-256 of exact SQL
bytes and apply DDL with its ledger row atomically. Tests observed that failed DDL
did not leave its table. Migration SQL is trusted owner-supplied code, never user input.
Use a disposable empty database for integration tests; verify_infrastructure.py
creates one in its own Compose project.

Dependencies read: [pgx pool](https://pkg.go.dev/github.com/jackc/pgx/v5/pgxpool),
[SQLx transactions](https://docs.rs/sqlx/0.8.6/sqlx/struct.Transaction.html),
[node-postgres transactions](https://node-postgres.com/features/transactions).
Versions: pgx 5.11.0, SQLx 0.8.6 (Rust 1.86 compatible), pg 8.23.0.
