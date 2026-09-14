use crate::{
    domains::audit::{self, Store as AuditStore},
    domains::org::infra::postgres::{invitation_migration, migration as org_migration},
    shared::{
        clock::SystemClock,
        errors::Failure,
        events::{
            Publisher,
            jetstream::Broker,
            postgres::{Mailbox, Store},
        },
        http::axum::{Admit, Route},
        id::V7,
        logger::{Fields, Logger},
        postgres::Database,
    },
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
use tokio::task::JoinHandle;

pub struct Runtime {
    store: Store,
    mailbox: Arc<Mailbox>,
    audit: Arc<AuditStore>,
    publisher: Arc<dyn Publisher>,
    broker: Option<Arc<Broker>>,
    ids: Mutex<V7<Box<dyn Fn() -> SystemTime + Send + Sync>>>,
    clock: Arc<SystemClock>,
    consumer: String,
    interval: Duration,
    log: Logger,
    stop: tokio::sync::watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

pub async fn compose(
    mut config: super::http::EventsConfig,
    consumer: String,
    database: Arc<Database>,
    clock: Arc<SystemClock>,
    admission: Admit,
    log: Logger,
) -> Result<(Route, Arc<Runtime>), Failure> {
    database
        .migrate(vec![
            crate::shared::events::postgres::migration(1),
            crate::domains::identity::infra::postgres::migration(2),
            crate::shared::events::postgres::receipts_migration(3),
            audit::migration(4),
            crate::domains::identity::infra::postgres::upgrade_ticket_migration(5),
            org_migration(6),
            invitation_migration(7),
        ])
        .await?;
    let mailbox = Arc::new(Mailbox {
        database: database.clone(),
    });
    let mut broker = None;
    let publisher: Arc<dyn Publisher> = if let Some(broker_config) = config.broker.take() {
        let opened = Arc::new(Broker::open(broker_config).await?);
        opened.provision().await?;
        broker = Some(opened.clone());
        opened
    } else {
        mailbox.clone()
    };
    let id_clock = clock.clone();
    let ids =
        V7::new(Box::new(move || id_clock.now()) as Box<dyn Fn() -> SystemTime + Send + Sync>);
    let (stop, _) = tokio::sync::watch::channel(false);
    let runtime = Arc::new(Runtime {
        store: Store {
            database: database.clone(),
        },
        mailbox: mailbox.clone(),
        audit: Arc::new(AuditStore { database }),
        publisher,
        broker,
        ids: Mutex::new(ids),
        clock,
        consumer,
        interval: config.interval,
        log,
        stop,
        task: Mutex::new(None),
    });
    Ok((audit::route(admission, runtime.audit.clone()), runtime))
}

impl Runtime {
    pub fn start(self: &Arc<Self>) {
        let runtime = self.clone();
        let mut stop = self.stop.subscribe();
        let task = tokio::spawn(async move {
            loop {
                runtime.tick().await;
                tokio::select! {
                    _ = tokio::time::sleep(runtime.interval) => {},
                    _ = stop.changed() => return,
                }
            }
        });
        *self.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
    }
    pub async fn close(&self, budget: Duration) {
        let _ = self.stop.send(true);
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            let _ = tokio::time::timeout(budget, task).await;
        }
        if let Some(broker) = &self.broker {
            broker.close().await;
        }
    }
    async fn tick(&self) {
        let token = match self.ids.lock().unwrap_or_else(|e| e.into_inner()).new_id() {
            Ok(v) => v,
            Err(e) => {
                self.log.with_failure(&e).warn(
                    "audit.worker.failed",
                    Fields::from([("stage".into(), "id".into())]),
                );
                return;
            }
        };
        match self.store.dispatch(&*self.publisher, token).await {
            Ok(_) => {}
            Err(e) => {
                self.log.with_failure(&e).warn(
                    "audit.worker.failed",
                    Fields::from([("stage".into(), "dispatch".into())]),
                );
                return;
            }
        }
        if let Some(broker) = &self.broker {
            if let Err(e) = broker.transfer(&*self.mailbox).await {
                self.log.with_failure(&e).warn(
                    "audit.worker.failed",
                    Fields::from([("stage".into(), "transfer".into())]),
                );
                return;
            }
        }
        let audit = self.audit.clone();
        let recorded = match self.clock.now().duration_since(std::time::UNIX_EPOCH) {
            Ok(v) => v.as_millis() as i64,
            Err(_) => return,
        };
        if let Err(e) = self
            .mailbox
            .consume(&self.consumer, move |tx, event| {
                let audit = audit.clone();
                Box::pin(async move {
                    if let Some(record) = audit::translate(&event, recorded)? {
                        audit.insert(tx, &record).await?;
                    }
                    Ok(())
                })
            })
            .await
        {
            self.log.with_failure(&e).warn(
                "audit.worker.failed",
                Fields::from([("stage".into(), "consume".into())]),
            );
        }
    }
}
