use futures_util::future::BoxFuture;
use n2f_rs::shared::{
    errors::{Classified, Failure, Kind},
    events::{
        Envelope, Publisher, Receipt,
        postgres::{Mailbox, Store, enqueue, migration},
    },
    id::Id,
    postgres::{Config, Database, Migration, map},
    secret::SecretString,
};
use std::{sync::Arc, time::Duration};
fn ident(n: u32) -> Id {
    Id::parse(&format!("00000000-0000-4000-8000-{n:012}")).unwrap()
}
fn event(n: u32) -> Envelope {
    let mut v: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../src/shared/events/fixtures/envelope.json"
    ))
    .unwrap();
    v["id"] = ident(n).to_string().into();
    Envelope::decode(&serde_json::to_vec(&v).unwrap()).unwrap()
}
async fn exec(db: &Database, sql: &str) {
    let sql = sql.to_owned();
    db.transaction(move |tx| {
        Box::pin(async move {
            use sqlx::Executor;
            tx.execute(sql.as_str()).await.map_err(map)?;
            Ok(())
        })
    })
    .await
    .unwrap();
}
async fn count(db: &Database, sql: &str) -> i64 {
    let sql = sql.to_owned();
    db.transaction(move |tx| {
        Box::pin(async move { sqlx::query_scalar(&sql).fetch_one(tx).await.map_err(map) })
    })
    .await
    .unwrap()
}
async fn ready(db: &Database) {
    exec(db,"UPDATE n2f_outbox SET available_at=clock_timestamp(); UPDATE n2f_mailbox SET available_at=clock_timestamp()").await;
}
async fn add(db: &Database, e: Envelope) {
    db.transaction(move |tx| Box::pin(async move { enqueue(tx, &e).await }))
        .await
        .unwrap();
}
struct Offline;
impl Publisher for Offline {
    fn publish<'a>(&'a self, _: &'a Envelope) -> BoxFuture<'a, Result<Receipt, Failure>> {
        Box::pin(async { Err(Failure::new(Kind::Unavailable, "offline")) })
    }
}
struct Wrong;
impl Publisher for Wrong {
    fn publish<'a>(&'a self, _: &'a Envelope) -> BoxFuture<'a, Result<Receipt, Failure>> {
        Box::pin(async {
            Ok(Receipt {
                event_id: ident(999),
                durable: true,
            })
        })
    }
}
#[tokio::test]
#[ignore = "requires N2F_TEST_DATABASE_URL pointing at a disposable PostgreSQL 18 database"]
async fn outbox_mailbox_integration() {
    let raw = std::env::var("N2F_TEST_DATABASE_URL").expect("explicit test database required");
    let db = Arc::new(
        Database::open(Config {
            url: SecretString::new(raw),
            max_connections: 4,
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap(),
    );
    db.migrate(vec![
        migration(1),
        Migration {
            version: 2,
            sql: "CREATE TABLE event_fixture(id integer PRIMARY KEY)".into(),
        },
    ])
    .await
    .unwrap();
    let rolled: Result<(), Failure> = db
        .transaction(|tx| {
            Box::pin(async move {
                sqlx::query("INSERT INTO event_fixture VALUES(1)")
                    .execute(&mut *tx)
                    .await
                    .map_err(map)?;
                enqueue(tx, &event(1)).await?;
                Err(Failure::new(Kind::Conflict, "fixture refusal"))
            })
        })
        .await;
    assert!(rolled.is_err());
    assert_eq!(count(&db, "SELECT count(*) FROM n2f_outbox").await, 0);
    assert_eq!(count(&db, "SELECT count(*) FROM event_fixture").await, 0);
    add(&db, event(1)).await;
    let store = Arc::new(Store {
        database: db.clone(),
    });
    let mailbox = Mailbox {
        database: db.clone(),
    };
    assert!(store.dispatch(&Offline, ident(100)).await.is_err());
    ready(&db).await;
    assert!(store.dispatch(&Wrong, ident(101)).await.is_err());
    ready(&db).await;
    assert!(store.dispatch(&mailbox, ident(102)).await.unwrap());
    mailbox.publish(&event(1)).await.unwrap();
    assert_eq!(count(&db, "SELECT count(*) FROM n2f_mailbox").await, 1);
    let first = event(1);
    let changed = Envelope::new(
        first.id(),
        "diagnostic.created.v1",
        1000,
        first.work(),
        serde_json::json!({"different":true}),
    )
    .unwrap();
    assert_eq!(
        mailbox
            .publish(&changed)
            .await
            .err()
            .unwrap()
            .classification()
            .unwrap()
            .kind,
        Kind::Conflict
    );
    assert!(
        mailbox
            .consume(|tx, _| Box::pin(async move {
                sqlx::query("INSERT INTO event_fixture VALUES(2)")
                    .execute(tx)
                    .await
                    .map_err(map)?;
                Err(Failure::new(Kind::Conflict, "fixture refusal"))
            }))
            .await
            .is_err()
    );
    assert_eq!(count(&db, "SELECT count(*) FROM event_fixture").await, 0);
    ready(&db).await;
    assert!(
        mailbox
            .consume(|tx, _| Box::pin(async move {
                sqlx::query("INSERT INTO event_fixture VALUES(2)")
                    .execute(tx)
                    .await
                    .map_err(map)?;
                Ok(())
            }))
            .await
            .unwrap()
    );
    assert_eq!(count(&db, "SELECT count(*) FROM event_fixture").await, 1);
    mailbox.publish(&event(1)).await.unwrap();
    assert!(
        !mailbox
            .consume(|_, _| Box::pin(async { panic!("duplicate consumed") }))
            .await
            .unwrap()
    );
    add(&db, event(2)).await;
    let old = store.claim(ident(200)).await.unwrap().unwrap();
    exec(&db,"UPDATE n2f_outbox SET lease_until=clock_timestamp()-interval '1 second' WHERE state='pending'").await;
    let fresh = store.claim(ident(201)).await.unwrap().unwrap();
    assert_eq!(
        store
            .ack(&old)
            .await
            .unwrap_err()
            .classification()
            .unwrap()
            .error_type,
        Some("events.lease_lost")
    );
    mailbox.publish(&fresh.event).await.unwrap();
    store.ack(&fresh).await.unwrap();
    add(&db, event(3)).await;
    for n in 0..5 {
        ready(&db).await;
        assert!(store.dispatch(&Offline, ident(300 + n)).await.is_err());
    }
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM n2f_outbox WHERE state='dead' AND attempts=5"
        )
        .await,
        1
    );
    for _ in 0..5 {
        ready(&db).await;
        assert!(
            mailbox
                .consume(|_, _| Box::pin(async {
                    Err(Failure::new(Kind::Conflict, "fixture refusal"))
                }))
                .await
                .is_err()
        );
    }
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM n2f_mailbox WHERE state='dead' AND attempts=5"
        )
        .await,
        1
    );
    add(&db, event(5)).await;
    add(&db, event(6)).await;
    let a = store.clone();
    let b = store.clone();
    let (a, b) = tokio::join!(a.claim(ident(500)), b.claim(ident(501)));
    assert_ne!(
        a.unwrap().unwrap().event.id(),
        b.unwrap().unwrap().event.id()
    );
    // jsonb output spacing expands this valid wire beyond the envelope budget.
    exec(&db, "TRUNCATE n2f_outbox, n2f_mailbox").await;
    let large = Envelope::new(
        ident(700),
        "diagnostic.created.v1",
        1000,
        first.work(),
        serde_json::json!({"values": vec![0; 30000]}),
    )
    .unwrap();
    add(&db, large).await;
    assert!(store.dispatch(&mailbox, ident(701)).await.unwrap());
    assert!(
        mailbox
            .consume(|_, got| Box::pin(async move {
                assert_eq!(got.id(), ident(700));
                Ok(())
            }))
            .await
            .unwrap()
    );
    db.close(Duration::from_secs(1)).await.unwrap();
}
