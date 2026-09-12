use futures_util::future::BoxFuture;
use n2f_rs::shared::{
    errors::{Classified, Failure, Kind},
    events::{
        Envelope, Publisher, Receipt,
        jetstream::{Broker, Config as BrokerConfig},
        postgres::{Mailbox, Store, enqueue, migration},
    },
    id::Id,
    postgres::{Config, Database, Migration, map},
    secret::SecretString,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
struct Wrong;
impl Publisher for Wrong {
    fn publish<'a>(&'a self, e: &'a Envelope) -> BoxFuture<'a, Result<Receipt, Failure>> {
        Box::pin(async move {
            Ok(Receipt {
                event_id: e.id(),
                durable: false,
            })
        })
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and JetStream"]
async fn jetstream_seam() {
    let database = Arc::new(
        Database::open(Config {
            url: SecretString::new(std::env::var("N2F_TEST_DATABASE_URL").unwrap()),
            max_connections: 4,
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap(),
    );
    database
        .migrate(vec![
            migration(1),
            Migration {
                version: 2,
                sql: "CREATE TABLE jetstream_effect(id uuid PRIMARY KEY)".into(),
            },
        ])
        .await
        .unwrap();
    let broker = Broker::open(BrokerConfig {
        url: SecretString::new(std::env::var("N2F_TEST_NATS_URL").unwrap()),
        stream: std::env::var("N2F_TEST_NATS_STREAM").unwrap(),
        consumer: "mailbox".into(),
        timeout: Duration::from_secs(1),
    })
    .await
    .unwrap();
    broker.provision().await.unwrap();
    broker.provision().await.unwrap();
    let event = Envelope::decode(include_bytes!(
        "../src/shared/events/fixtures/envelope.json"
    ))
    .unwrap();
    let a = Id::parse("00000000-0000-4000-8000-000000000010").unwrap();
    let b = Id::parse("00000000-0000-4000-8000-000000000002").unwrap();
    let store = Store {
        database: database.clone(),
    };
    let mailbox = Mailbox {
        database: database.clone(),
    };
    let e = event.clone();
    let rollback: Result<(), Failure> = database
        .transaction(move |tx| {
            Box::pin(async move {
                enqueue(tx, &e).await?;
                Err(Failure::new(Kind::Conflict, "fixture refusal"))
            })
        })
        .await;
    assert!(rollback.is_err());
    assert!(!store.dispatch(&broker, a).await.unwrap());
    let e = event.clone();
    database
        .transaction(move |tx| Box::pin(async move { enqueue(tx, &e).await }))
        .await
        .unwrap();
    let lease = store.claim(a).await.unwrap().unwrap();
    let receipt = broker.publish(&lease.event).await.unwrap();
    assert!(receipt.durable);
    assert_eq!(receipt.event_id, event.id());
    database
        .transaction(|tx| {
            Box::pin(async move {
                sqlx::query(
                    "UPDATE n2f_outbox SET lease_until=clock_timestamp()-interval '1 second'",
                )
                .execute(tx)
                .await
                .map_err(map)?;
                Ok(())
            })
        })
        .await
        .unwrap();
    assert!(store.dispatch(&broker, b).await.unwrap());
    let changed = Envelope::new(
        event.id(),
        "diagnostic.created.v1",
        1000,
        event.work(),
        serde_json::json!({"changed":true}),
    )
    .unwrap();
    assert_eq!(
        broker
            .publish(&changed)
            .await
            .err()
            .unwrap()
            .classification()
            .unwrap()
            .kind,
        Kind::Conflict
    );
    assert!(broker.transfer(&Wrong).await.is_err());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(broker.transfer(&mailbox).await.unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    assert!(
        mailbox
            .consume(move |tx, e| Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                sqlx::query("INSERT INTO jetstream_effect VALUES($1::uuid)")
                    .bind(e.id().to_string())
                    .execute(tx)
                    .await
                    .map_err(map)?;
                Ok(())
            }))
            .await
            .unwrap()
    );
    mailbox.publish(&event).await.unwrap();
    assert!(
        !mailbox
            .consume(|_, _| Box::pin(async { panic!("duplicate effects") }))
            .await
            .unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!broker.transfer(&mailbox).await.unwrap());
    let base = Envelope::new(
        a,
        "diagnostic.large.v1",
        1000,
        event.work(),
        serde_json::json!({"value":""}),
    )
    .unwrap();
    let large = Envelope::new(
        a,
        "diagnostic.large.v1",
        1000,
        event.work(),
        serde_json::json!({"value":"x".repeat(65536-base.bytes().len())}),
    )
    .unwrap();
    assert_eq!(large.bytes().len(), 65536);
    broker.publish(&large).await.unwrap();
    assert!(broker.transfer(&mailbox).await.unwrap());
    assert!(
        mailbox
            .consume(|_, _| Box::pin(async { Ok(()) }))
            .await
            .unwrap()
    );
    database
        .transaction(move |tx| Box::pin(async move { enqueue(tx, &large).await }))
        .await
        .unwrap();
    broker.close().await;
    assert!(store.dispatch(&broker, b).await.is_err());
    let state: String = database
        .transaction(move |tx| {
            Box::pin(async move {
                sqlx::query_scalar("SELECT state FROM n2f_outbox WHERE event_id=$1::uuid")
                    .bind(a.to_string())
                    .fetch_one(tx)
                    .await
                    .map_err(map)
            })
        })
        .await
        .unwrap();
    assert_eq!(state, "pending");
    assert!(broker.publish(&event).await.is_err());
    database.close(Duration::from_secs(1)).await.unwrap();
}
