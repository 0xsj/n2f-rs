use n2f_rs::shared::{
    errors::{Classified, Failure, Kind},
    postgres::{Config, Database, Migration, map},
    secret::SecretString,
};
use std::{sync::Arc, time::Duration};
#[tokio::test]
#[ignore = "requires N2F_TEST_DATABASE_URL pointing at disposable PostgreSQL 18"]
async fn database_integration() {
    let raw = std::env::var("N2F_TEST_DATABASE_URL")
        .expect("explicit disposable test database URL required");
    let db = Arc::new(
        Database::open(Config {
            url: SecretString::new(raw),
            max_connections: 4,
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap(),
    );
    let ms = vec![Migration {
        version: 1,
        sql: "CREATE TABLE n2f_fixture (id integer PRIMARY KEY, value text NOT NULL)".into(),
    }];
    db.migrate(ms.clone()).await.unwrap();
    db.migrate(ms.clone()).await.unwrap();
    let mut changed = ms.clone();
    changed[0].sql.push(' ');
    assert_eq!(
        db.migrate(changed)
            .await
            .unwrap_err()
            .classification()
            .unwrap()
            .error_type,
        Some("database.migration_drift")
    );
    let mut bad = ms.clone();
    bad.push(Migration {
        version: 2,
        sql: "CREATE TABLE n2f_rolled_migration (id int); SELECT 1/0".into(),
    });
    assert!(db.migrate(bad).await.is_err());
    let exists: Option<String> = db
        .transaction(|tx| {
            Box::pin(async move {
                sqlx::query_scalar("SELECT to_regclass('public.n2f_rolled_migration')::text")
                    .fetch_one(tx)
                    .await
                    .map_err(map)
            })
        })
        .await
        .unwrap();
    assert!(exists.is_none());
    db.transaction(|tx| {
        Box::pin(async move {
            sqlx::query("INSERT INTO n2f_fixture VALUES(1,'committed')")
                .execute(tx)
                .await
                .map_err(map)?;
            Ok(())
        })
    })
    .await
    .unwrap();
    let failed: Result<(), Failure> = db
        .transaction(|tx| {
            Box::pin(async move {
                sqlx::query("INSERT INTO n2f_fixture VALUES(2,'rollback')")
                    .execute(tx)
                    .await
                    .map_err(map)?;
                Err(Failure::new(Kind::Conflict, "fixture refusal"))
            })
        })
        .await;
    assert!(failed.is_err());
    // PostgreSQL COMMIT on an aborted transaction may return a ROLLBACK command tag.
    let swallowed = db
        .transaction(|tx| {
            Box::pin(async move {
                let _ = sqlx::query("SELECT 1/0").execute(tx).await;
                Ok(())
            })
        })
        .await;
    assert!(
        swallowed.is_err(),
        "swallowed statement must not claim commit"
    );
    let mut tasks = Vec::new();
    for id in 10..18 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db.transaction(move |tx| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO n2f_fixture VALUES($1,'parallel')")
                        .bind(id)
                        .execute(tx)
                        .await
                        .map_err(map)?;
                    Ok(())
                })
            })
            .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let count: i64 = db
        .transaction(|tx| {
            Box::pin(async move {
                sqlx::query_scalar("SELECT count(*) FROM n2f_fixture")
                    .fetch_one(tx)
                    .await
                    .map_err(map)
            })
        })
        .await
        .unwrap();
    assert_eq!(count, 9);
    let start = std::time::Instant::now();
    assert!(
        db.transaction(|tx| Box::pin(async move {
            sqlx::query("SELECT pg_sleep(2)")
                .execute(tx)
                .await
                .map_err(map)?;
            Ok(())
        }))
        .await
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    db.close(Duration::from_secs(1)).await.unwrap();
    assert!(db.ping().await.is_err());
}
