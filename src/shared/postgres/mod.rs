//! SQLx pooling and explicit infrastructure transactions. See CONTRACT.md.
use crate::shared::{
    errors::{Failure, Kind},
    secret::SecretString,
};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use sqlx::{
    ConnectOptions, Executor, PgConnection, PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub struct Config {
    pub url: SecretString,
    pub max_connections: u32,
    pub timeout: Duration,
}
pub struct Database {
    pool: PgPool,
    budget: Duration,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid database configuration").with_type("database.config")
}
pub fn map(error: sqlx::Error) -> Failure {
    let (kind, code) = match &error {
        sqlx::Error::Database(e) => match e.code().as_deref() {
            Some("23505" | "40001" | "40P01") => (Kind::Conflict, "database.conflict"),
            Some("57014") => (Kind::Timeout, "database.timeout"),
            _ => (Kind::Internal, "database.failed"),
        },
        sqlx::Error::PoolTimedOut => (Kind::Timeout, "database.timeout"),
        _ => (Kind::Unavailable, "database.unavailable"),
    };
    Failure::new(kind, "database operation failed").with_type(code)
}
pub fn commit_failure(error: sqlx::Error) -> Failure {
    if matches!(error, sqlx::Error::Database(_)) {
        map(error)
    } else {
        uncertain()
    }
}
fn uncertain() -> Failure {
    Failure::new(Kind::Unavailable, "database commit outcome uncertain")
        .with_type("database.commit_uncertain")
}
fn timeout() -> Failure {
    Failure::new(Kind::Timeout, "database operation timed out").with_type("database.timeout")
}
impl Database {
    pub async fn open(c: Config) -> Result<Self, Failure> {
        let u = url::Url::parse(c.url.reveal()).map_err(|_| invalid())?;
        if !matches!(u.scheme(), "postgres" | "postgresql")
            || u.host_str().is_none()
            || u.fragment().is_some()
            || !(1..=64).contains(&c.max_connections)
            || c.timeout < Duration::from_millis(1)
            || c.timeout > Duration::from_secs(30)
        {
            return Err(invalid());
        }
        let options = PgConnectOptions::from_str(c.url.reveal())
            .map_err(|_| invalid())?
            .disable_statement_logging()
            .options([
                ("statement_timeout", c.timeout.as_millis().to_string()),
                (
                    "idle_in_transaction_session_timeout",
                    c.timeout.as_millis().to_string(),
                ),
            ]);
        let pool = tokio::time::timeout(
            c.timeout,
            PgPoolOptions::new()
                .max_connections(c.max_connections)
                .acquire_timeout(c.timeout)
                .connect_with(options),
        )
        .await
        .map_err(|_| timeout())?
        .map_err(map)?;
        Ok(Self {
            pool,
            budget: c.timeout,
        })
    }
    pub async fn ping(&self) -> Result<(), Failure> {
        tokio::time::timeout(self.budget, sqlx::query("SELECT 1").execute(&self.pool))
            .await
            .map_err(|_| timeout())?
            .map_err(map)?;
        Ok(())
    }
    /// The borrowed connection cannot be retained by safe Rust. No manual COMMIT,
    /// detached work or external effects inside the callback.
    pub async fn transaction<T: Send>(
        &self,
        f: impl for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, Failure>>,
    ) -> Result<T, Failure> {
        let committing = AtomicBool::new(false);
        let operation = async {
            let mut tx = self.pool.begin().await.map_err(map)?;
            let value = f(&mut tx).await?;
            // SQLx commit discards the server command tag. Probe transaction state
            // before COMMIT so a swallowed statement error cannot become success.
            sqlx::query("SELECT 1")
                .execute(&mut *tx)
                .await
                .map_err(map)?;
            committing.store(true, Ordering::SeqCst);
            tx.commit().await.map_err(commit_failure)?;
            Ok(value)
        };
        tokio::time::timeout(self.budget, operation)
            .await
            .map_err(|_| {
                if committing.load(Ordering::SeqCst) {
                    uncertain()
                } else {
                    timeout()
                }
            })?
    }
    pub async fn close(&self, budget: Duration) -> Result<(), Failure> {
        tokio::time::timeout(budget.min(self.budget), self.pool.close())
            .await
            .map_err(|_| timeout())
    }
    pub async fn migrate(&self, migrations: Vec<Migration>) -> Result<(), Failure> {
        let mut previous = 0;
        for m in &migrations {
            if m.version <= previous || m.sql.is_empty() {
                return Err(invalid());
            }
            previous = m.version;
        }
        self.transaction(move |tx| Box::pin(async move {
            sqlx::query("SELECT pg_advisory_xact_lock(925005)").execute(&mut *tx).await.map_err(map)?;
            sqlx::query("CREATE TABLE IF NOT EXISTS public.n2f_migrations (version bigint PRIMARY KEY, checksum text NOT NULL)").execute(&mut *tx).await.map_err(map)?;
            let rows=sqlx::query("SELECT version,checksum FROM public.n2f_migrations ORDER BY version").fetch_all(&mut *tx).await.map_err(map)?;
            if rows.len()>migrations.len() { return Err(drift()); }
            for (i,m) in migrations.iter().enumerate() {
                let checksum=format!("{:x}",Sha256::digest(m.sql.as_bytes()));
                if let Some(row)=rows.get(i) {
                    if row.get::<i64,_>("version")!=m.version || row.get::<String,_>("checksum")!=checksum { return Err(drift()); }
                } else {
                    tx.execute(m.sql.as_str()).await.map_err(map)?;
                    sqlx::query("INSERT INTO public.n2f_migrations(version,checksum) VALUES ($1,$2)").bind(m.version).bind(checksum).execute(&mut *tx).await.map_err(map)?;
                }
            }
            Ok(())
        })).await
    }
}
#[derive(Clone)]
pub struct Migration {
    pub version: i64,
    pub sql: String,
}
fn drift() -> Failure {
    Failure::new(Kind::Conflict, "migration history differs").with_type("database.migration_drift")
}
