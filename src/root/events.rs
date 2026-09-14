//! Finite producer/dispatcher/consumer composition, separate from future domains.
use super::{config::load_config, logging::logging};
use crate::shared::{
    clock::SystemClock,
    env::Reader,
    errors::{Failure, Kind},
    events::{
        Envelope, Publisher,
        jetstream::{Broker, Config as BrokerConfig},
        postgres::{Mailbox, Store, enqueue, migration, receipts_migration},
    },
    id::V7,
    logger,
    postgres::{Config, Database},
    provenance as p,
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
pub fn run(lookup: impl Fn(&str) -> Option<String>) -> i32 {
    let Ok(mut c) = load_config(&lookup) else {
        return 2;
    };
    let mut r = Reader::new(lookup);
    let config = Config {
        url: r.secret("DATABASE_URL"),
        max_connections: r.int("DATABASE_MAX_CONNECTIONS", 8, 1, 64) as u32,
        timeout: Duration::from_millis(r.int("DATABASE_TIMEOUT_MS", 1000, 1, 30000) as u64),
    };
    let transport = r.string("EVENTS_TRANSPORT", "postgres");
    let broker_config = if transport == "jetstream" {
        Some(BrokerConfig {
            url: r.secret("NATS_URL"),
            stream: r.string("NATS_STREAM", "N2F_EVENTS"),
            consumer: r.string("NATS_CONSUMER", "mailbox"),
            timeout: Duration::from_millis(r.int("NATS_TIMEOUT_MS", 1000, 1, 5000) as u64),
        })
    } else {
        None
    };
    if r.check().is_err() || !matches!(transport.as_str(), "postgres" | "jetstream") {
        return 2;
    }
    let clock = Arc::new(SystemClock::new());
    let id_clock = clock.clone();
    let mut ids = V7::new(move || id_clock.now());
    let Ok(instance) = ids.new_id() else {
        return 1;
    };
    c.resource.instance_id = instance.to_string();
    let Ok(log) = logging(&c, clock.clone(), Box::new(std::io::stdout()), false) else {
        return 1;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        let broker = if let Some(config) = broker_config {
            Some(Broker::open(config).await?)
        } else {
            None
        };
        let database = Arc::new(Database::open(config).await?);
        let db = database.clone();
        let operation = async {
            if let Some(b) = &broker {
                b.provision().await?;
            }
            db.migrate(vec![migration(1), receipts_migration(3)])
                .await?;
            let event_id = ids.new_id()?;
            let factory_clock = clock.clone();
            let mut factory = p::Factory::new(move || factory_clock.now(), move || ids.new_id());
            let scope = factory.open(p::RootSpec {
                work_id: None,
                origin: p::Origin::Startup,
                operation: p::Operation::new("events.example".into())?,
                attribution: p::Attribution::default(),
                executor: p::Actor::new(p::ActorKind::Service, c.resource.name.clone())?,
            })?;
            let scoped = log.log.with_scope(&scope);
            let event = Envelope::new(
                event_id,
                "diagnostic.created.v1",
                clock
                    .now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| Failure::new(Kind::Invalid, "invalid event time"))?
                    .as_millis() as i64,
                scope.work_context(),
                serde_json::json!({"example":true}),
            )?;
            db.transaction(move |tx| Box::pin(async move { enqueue(tx, &event).await }))
                .await?;
            scoped.info(
                "events.enqueued",
                BTreeMap::from([("event_id".into(), event_id.to_string().into())]),
            );
            let store = Store {
                database: db.clone(),
            };
            let mailbox = Mailbox {
                database: db.clone(),
            };
            let publisher: &dyn Publisher = match &broker {
                Some(b) => b,
                None => &mailbox,
            };
            scoped.info(
                "events.transport.selected",
                BTreeMap::from([("transport".into(), transport.clone().into())]),
            );
            for _ in 0..32 {
                let token = factory
                    .open(p::RootSpec {
                        work_id: None,
                        origin: p::Origin::Startup,
                        operation: p::Operation::new("events.dispatch".into())?,
                        attribution: p::Attribution::default(),
                        executor: p::Actor::new(p::ActorKind::Service, c.resource.name.clone())?,
                    })?
                    .snapshot()
                    .scope_id;
                let found = store.dispatch(publisher, token).await?;
                scoped.info(
                    "events.dispatch.completed",
                    BTreeMap::from([("found".into(), logger::Value::Bool(found))]),
                );
                let seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
                if let Some(b) = &broker {
                    let transferred = b.transfer(&mailbox).await?;
                    scoped.info(
                        "events.transfer.completed",
                        BTreeMap::from([("found".into(), logger::Value::Bool(transferred))]),
                    );
                }
                let mark = seen.clone();
                let consumed = mailbox
                    .consume("events-example", move |_, received| {
                        Box::pin(async move {
                            mark.store(
                                received.id() == event_id,
                                std::sync::atomic::Ordering::SeqCst,
                            );
                            Ok(())
                        })
                    })
                    .await?;
                scoped.info(
                    "events.consume.completed",
                    BTreeMap::from([("found".into(), logger::Value::Bool(consumed))]),
                );
                if seen.load(std::sync::atomic::Ordering::SeqCst) {
                    scoped.info(
                        "events.example.completed",
                        BTreeMap::from([("event_id".into(), event_id.to_string().into())]),
                    );
                    return Ok::<(), Failure>(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(Failure::new(
                Kind::Timeout,
                "diagnostic delivery budget exhausted",
            ))
        };
        let result = tokio::time::timeout(Duration::from_secs(10), operation)
            .await
            .map_err(|_| Failure::new(Kind::Timeout, "event example timed out"))
            .and_then(|v| v);
        let _ = database.close(Duration::from_secs(2)).await;
        if let Some(b) = broker {
            b.close().await;
        }
        result
    });
    let code = match result {
        Ok(()) => 0,
        Err(e) => {
            log.log
                .with_failure(&e)
                .error("events.example.failed", BTreeMap::new());
            1
        }
    };
    drop(runtime);
    let _ = log.close(Duration::from_secs(2));
    code
}
