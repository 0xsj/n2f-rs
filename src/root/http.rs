use super::{
    audit::compose as compose_audit,
    auth::{AuthConfig, compose as compose_auth, load_auth},
    config::{Config, load_config},
    logging::logging,
    proxy::{TrustedProxy, parse_trusted_proxies, trusted_source},
};
use crate::domains::audit::migration as audit_migration;
use crate::domains::identity::transport::http::WebSocketAdmission;
use crate::shared::{
    clock::SystemClock,
    env::{Reader, Var},
    errors::{Failure, Kind},
    events::jetstream::Config as BrokerConfig,
    health::{Check, Gate},
    http::{
        axum::{Config as ServerConfig, RequestFailure, Route, Server, json_work},
        otel::Observer,
    },
    httpclient::{
        Client as NativeClient, Config as ClientConfig, Request as OutboundRequest,
        otel::Client as ObservedClient,
    },
    id::V7,
    postgres::{Config as DatabaseConfig, Database},
    provenance as p,
    socket::{
        Message as SocketMessage,
        axum::{
            Admission as SocketAdmission, Authorize as SocketAuthorize,
            Revalidate as SocketRevalidate, Server as SocketServer, Session as SocketSession,
        },
    },
    telemetry::otel::{Config as TelemetryConfig, Runtime},
};
use axum::{Router, body::Body, extract::State, http::Request};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tracing_subscriber::{Layer, layer::SubscriberExt};
pub struct HTTPConfig {
    pub auth: Option<AuthConfig>,
    pub manifest: Vec<Var>,
    pub socket_origin: String,
    pub outbound_origin: String,
    pub database: Option<DatabaseConfig>,
    pub test_routes: bool,
    pub base: Config,
    pub host: IpAddr,
    pub port: u16,
    pub timeout: Duration,
    pub shutdown: Duration,
    pub telemetry: TelemetryConfig,
    pub events: EventsConfig,
    pub audit_enabled: bool,
    pub audit_consumer: String,
    pub trusted_proxies: Vec<TrustedProxy>,
}
pub struct EventsConfig {
    pub transport: String,
    pub broker: Option<BrokerConfig>,
    pub interval: Duration,
}
pub fn load_http_config(lookup: impl Fn(&str) -> Option<String>) -> Result<HTTPConfig, Failure> {
    let base = load_config(&lookup)?;
    // A password without a username must be refused, not ignored (AUTH_BUILD.md);
    // presence is taken from the raw lookup so the value is never recorded.
    let smtp_password_present = lookup("AUTH_SMTP_PASSWORD").is_some_and(|v| !v.is_empty());
    let mut r = Reader::new(lookup);
    let host = r
        .string("HTTP_HOST", "127.0.0.1")
        .parse()
        .map_err(|_| Failure::new(Kind::Invalid, "invalid HTTP host").with_type("env.invalid"))?;
    let port = r.int("HTTP_PORT", 7200, 0, 65535) as u16;
    let timeout = Duration::from_millis(r.int("HTTP_TIMEOUT_MS", 1000, 1, 30000) as u64);
    let shutdown = Duration::from_millis(r.int("SHUTDOWN_MS", 3000, 1, 30000) as u64);
    let test_routes = r.boolean("HTTP_TEST_ROUTES", false);
    let outbound_origin = r.string("OUTBOUND_ORIGIN", "");
    let socket_origin = r.string("WS_ORIGIN", "http://localhost:3000");
    let trusted_proxies = parse_trusted_proxies(&r.string("HTTP_TRUSTED_PROXIES", ""))?;
    let telemetry = TelemetryConfig {
        mode: r.enumeration("TELEMETRY_MODE", "none", &["none", "otlp"]),
        endpoint: r.string("TELEMETRY_ENDPOINT", "http://127.0.0.1:7242"),
        sampling: r.enumeration("TELEMETRY_SAMPLING", "all", &["all", "none"]),
        queue: r.int("TELEMETRY_QUEUE", 256, 1, 65536) as usize,
        batch: r.int("TELEMETRY_BATCH", 64, 1, 65536) as usize,
        timeout: Duration::from_millis(r.int("TELEMETRY_TIMEOUT_MS", 500, 1, 10000) as u64),
        interval: Duration::from_millis(r.int("TELEMETRY_INTERVAL_MS", 500, 100, 60000) as u64),
        resource: BTreeMap::from([
            ("service.name".into(), base.resource.name.clone()),
            ("service.instance.id".into(), "validation".into()),
        ]),
    };
    let database = if r.boolean("DATABASE_ENABLED", false) {
        Some(DatabaseConfig {
            url: r.secret("DATABASE_URL"),
            max_connections: r.int("DATABASE_MAX_CONNECTIONS", 8, 1, 64) as u32,
            timeout: Duration::from_millis(r.int("DATABASE_TIMEOUT_MS", 1000, 1, 30000) as u64),
        })
    } else {
        None
    };
    let transport = r.enumeration("EVENTS_TRANSPORT", "postgres", &["postgres", "jetstream"]);
    let interval = Duration::from_millis(r.int("EVENTS_INTERVAL_MS", 100, 10, 60000) as u64);
    let broker = if transport == "jetstream" {
        Some(BrokerConfig {
            url: r.secret("NATS_URL"),
            stream: r.string("NATS_STREAM", "N2F_EVENTS"),
            consumer: r.string("NATS_CONSUMER", "mailbox"),
            timeout: Duration::from_millis(r.int("NATS_TIMEOUT_MS", 1000, 1, 5000) as u64),
        })
    } else {
        None
    };
    let auth = load_auth(&mut r, host, smtp_password_present)?;
    let audit_enabled = r.boolean("AUDIT_ENABLED", auth.is_some());
    let audit_consumer = r.string("AUDIT_CONSUMER", "audit");
    r.check()?;
    if auth.is_some() && database.is_none() {
        return Err(Failure::new(Kind::Invalid, "invalid configuration")
            .with_type("env.invalid")
            .with_field("AUTH_ENABLED", "requires DATABASE_ENABLED=true"));
    }
    if audit_enabled && auth.is_none() {
        return Err(Failure::new(Kind::Invalid, "invalid configuration")
            .with_type("env.invalid")
            .with_field("AUDIT_ENABLED", "requires AUTH_ENABLED=true"));
    }
    let manifest = r.manifest()?;
    telemetry.validate()?;
    Ok(HTTPConfig {
        auth,
        manifest,
        socket_origin,
        outbound_origin,
        database,
        test_routes,
        base,
        host,
        port,
        timeout,
        shutdown,
        telemetry,
        events: EventsConfig {
            transport,
            broker,
            interval,
        },
        audit_enabled,
        audit_consumer,
        trusted_proxies,
    })
}
struct Diagnostics(Arc<AtomicU64>);
impl<S: tracing::Subscriber> Layer<S> for Diagnostics {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if *event.metadata().level() <= tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}
pub fn run_http(lookup: impl Fn(&str) -> Option<String>) -> i32 {
    let mut c = match load_http_config(lookup) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("configuration refused");
            return 2;
        }
    };
    let clock = Arc::new(SystemClock::new());
    let id_clock = clock.clone();
    let mut ids = V7::new(move || id_clock.now());
    let instance = match ids.new_id() {
        Ok(i) => i.to_string(),
        Err(_) => return 1,
    };
    c.base.resource.instance_id = instance.clone();
    let log = match logging(
        &c.base,
        clock.clone(),
        Box::new(io::stdout()),
        io::stdout().is_terminal(),
    ) {
        Ok(l) => l,
        Err(_) => return 1,
    };
    c.telemetry.resource = BTreeMap::from([
        ("service.name".into(), c.base.resource.name.clone()),
        (
            "service.namespace".into(),
            c.base.resource.namespace.clone(),
        ),
        ("service.version".into(), c.base.resource.version.clone()),
        ("service.instance.id".into(), instance),
        (
            "deployment.environment.name".into(),
            c.base.resource.environment.clone(),
        ),
    ]);
    let events = Arc::new(AtomicU64::new(0));
    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(Diagnostics(events.clone())),
    );
    // The executable owns panic diagnostics; never print raw caught panic payloads.
    let panic_events = events.clone();
    std::panic::set_hook(Box::new(move |_| {
        panic_events.fetch_add(1, Ordering::Relaxed);
    }));
    let telemetry = match Runtime::new(c.telemetry) {
        Ok(r) => Arc::new(r),
        Err(_) => {
            eprintln!("telemetry startup failed");
            let _ = log.close(c.shutdown);
            return 1;
        }
    };
    let factory_clock = clock.clone();
    let factory = Arc::new(Mutex::new(p::Factory::new(
        move || factory_clock.now(),
        move || ids.new_id(),
    )));
    let executor = p::Actor::new(p::ActorKind::Service, c.base.resource.name.clone()).unwrap();
    let attribution = p::Attribution::new(p::AttributionSpec {
        initiator: Some(p::Actor::anonymous()),
        ..Default::default()
    })
    .unwrap();
    let socket_factory = factory.clone();
    let socket_executor = executor.clone();
    let socket_attribution = attribution.clone();
    let mut socket_authorize: Option<SocketAuthorize> = None;
    let mut socket_revalidate: Option<SocketRevalidate> = None;
    let open = Arc::new(
        move |name: &str, incoming: &p::IncomingResult, admitted: Option<p::Attribution>| {
            factory.lock().unwrap_or_else(|e| e.into_inner()).enter(
                p::RootSpec {
                    work_id: None,
                    origin: p::Origin::Request,
                    operation: p::Operation::new(name.into())?,
                    attribution: admitted.unwrap_or_else(|| attribution.clone()),
                    executor: executor.clone(),
                },
                incoming,
            )
        },
    );
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let database = if let Some(config) = c.database.take() {
        match tokio.block_on(Database::open(config)) {
            Ok(db) => Some(Arc::new(db)),
            Err(_) => {
                eprintln!("database startup failed");
                let _ = telemetry.close(c.shutdown);
                let _ = log.close(c.shutdown);
                return 1;
            }
        }
    } else {
        None
    };
    let mut audit_runtime: Option<Arc<super::audit::Runtime>> = None;
    let checks: Vec<Check> = database
        .as_ref()
        .map(|db| {
            let db = db.clone();
            vec![Arc::new(move || {
                let db = db.clone();
                Box::pin(async move { db.ping().await })
                    as futures_util::future::BoxFuture<'static, Result<(), Failure>>
            }) as Check]
        })
        .unwrap_or_default();
    let gate = Arc::new(Gate::new(Duration::from_millis(500), checks).unwrap());
    let readiness = gate.clone();
    let outbound_native = if c.outbound_origin.is_empty() {
        None
    } else {
        match NativeClient::new(ClientConfig {
            origin: c.outbound_origin.clone(),
            timeout: Duration::from_millis(500),
            max_request: 65536,
            max_response: 65536,
            bearer: None,
        }) {
            Ok(n) => Some(Arc::new(n)),
            Err(_) => {
                eprintln!("outbound startup failed");
                if let Some(db) = &database {
                    let _ = tokio.block_on(db.close(c.shutdown));
                }
                let _ = telemetry.close(c.shutdown);
                let _ = log.close(c.shutdown);
                return 1;
            }
        }
    };
    let outbound = outbound_native
        .as_ref()
        .map(|n| Arc::new(ObservedClient::new(n.clone(), telemetry.clone())));
    let mut routes = vec![
        Route {
            path: "/_examples/outbound".into(),
            operation: "http.example.outbound".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(move |_| {
                let outbound = outbound.clone();
                json_work(async move {
                    let client = outbound.ok_or_else(|| {
                        RequestFailure::Known(Failure::new(
                            Kind::Unavailable,
                            "outbound not configured",
                        ))
                    })?;
                    let r = client
                        .execute(OutboundRequest {
                            method: "GET".into(),
                            path: "/probe".into(),
                            body: vec![],
                            trace: None,
                        })
                        .await
                        .map_err(RequestFailure::Known)?;
                    Ok(serde_json::json!({"status":r.status,"body_bytes":r.body.len()}))
                })
            }),
        },
        Route {
            path: "/livez".into(),
            operation: "health.live".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(|_| json_work(async { Ok(serde_json::json!({"status":"alive"})) })),
        },
        Route {
            path: "/readyz".into(),
            operation: "health.ready".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(move |_| {
                let gate = readiness.clone();
                json_work(async move {
                    if gate.ready().await {
                        Ok(serde_json::json!({"status":"ready"}))
                    } else {
                        Err(RequestFailure::Known(
                            Failure::new(Kind::Unavailable, "not ready")
                                .with_type("health.not_ready"),
                        ))
                    }
                })
            }),
        },
        Route {
            path: "/_examples/http/success".into(),
            operation: "http.example.success".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(|_| json_work(async { Ok(serde_json::json!({"ok":true})) })),
        },
        Route {
            path: "/_examples/http/conflict".into(),
            operation: "http.example.conflict".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(|_| {
                json_work(async {
                    Err(RequestFailure::Known(
                        Failure::new(Kind::Conflict, "item already exists")
                            .with_type("demo.exists"),
                    ))
                })
            }),
        },
        Route {
            path: "/_examples/http/unavailable".into(),
            operation: "http.example.unavailable".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(|_| {
                json_work(async {
                    Err(RequestFailure::Known(
                        Failure::new(Kind::Unavailable, "dependency unavailable")
                            .with_type("demo.offline"),
                    ))
                })
            }),
        },
        Route {
            path: "/_examples/http/unknown".into(),
            operation: "http.example.unknown".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(|_| json_work(async { Err(RequestFailure::Unknown) })),
        },
    ];

    if c.test_routes {
        routes.push(Route {
            path: "/_examples/http/panic".into(),
            operation: "http.example.panic".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(|_| json_work(async { panic!("credential-SENTINEL") })),
        });
        let timeout = c.timeout;
        routes.push(Route {
            path: "/_examples/http/delay".into(),
            operation: "http.example.delay".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(move |_| {
                json_work(async move {
                    tokio::time::sleep(timeout * 2).await;
                    Ok(serde_json::json!({"ok":true}))
                })
            }),
        });
        routes.push(Route {
            path: "/_examples/http/context".into(),
            operation: "http.example.context".into(),
            method: "GET".into(),
            admission: None,
            handler: Arc::new(|_| {
                json_work(async {
                    use opentelemetry::trace::TraceContextExt;
                    let trace_before = opentelemetry::Context::current()
                        .span()
                        .span_context()
                        .clone();
                    let before = crate::shared::http::axum::current()
                        .unwrap()
                        .scope
                        .snapshot()
                        .scope_id
                        .to_string();
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    let after = crate::shared::http::axum::current()
                        .unwrap()
                        .scope
                        .snapshot()
                        .scope_id
                        .to_string();
                    let mut body = serde_json::json!({"before":before,"after":after});
                    if trace_before.is_valid() {
                        body["trace_before"] = trace_before.span_id().to_string().into();
                        body["trace_after"] = opentelemetry::Context::current()
                            .span()
                            .span_context()
                            .span_id()
                            .to_string()
                            .into();
                    }
                    Ok(body)
                })
            }),
        });
    }
    if let Some(auth) = &c.auth {
        let db = database.clone().expect("auth requires the database");
        match tokio.block_on(compose_auth(
            auth,
            db.clone(),
            clock.clone(),
            &c.manifest,
            log.log.clone(),
            (c.audit_enabled).then(|| audit_migration(4)),
        )) {
            Ok((auth_routes, manifest, admission, authorize, revalidate)) => {
                routes.extend(auth_routes);
                socket_authorize = Some(authorize);
                socket_revalidate = Some(revalidate);
                log.log.info("http.start", manifest);
                let event_config = EventsConfig {
                    transport: c.events.transport.clone(),
                    broker: c.events.broker.take(),
                    interval: c.events.interval,
                };
                if c.audit_enabled {
                    match tokio.block_on(compose_audit(
                        event_config,
                        c.audit_consumer.clone(),
                        db.clone(),
                        clock.clone(),
                        admission,
                        log.log.clone(),
                    )) {
                        Ok((route, runtime)) => {
                            routes.push(route);
                            audit_runtime = Some(runtime);
                        }
                        Err(_) => {
                            eprintln!("audit startup failed");
                            if let Some(db) = &database {
                                let _ = tokio.block_on(db.close(c.shutdown));
                            }
                            let _ = telemetry.close(c.shutdown);
                            let _ = log.close(c.shutdown);
                            return 1;
                        }
                    }
                }
            }
            Err(_) => {
                eprintln!("authentication startup failed");
                if let Some(db) = &database {
                    let _ = tokio.block_on(db.close(c.shutdown));
                }
                let _ = telemetry.close(c.shutdown);
                let _ = log.close(c.shutdown);
                return 1;
            }
        }
    }
    let trusted_proxies = c.trusted_proxies.clone();
    let adapter = Arc::new(
        Server::new(ServerConfig {
            routes,
            observer: Arc::new(Observer::new(telemetry.clone())),
            log: log.log.clone(),
            open,
            source: Arc::new(move |peer, headers| trusted_source(&trusted_proxies, peer, headers)),
            timeout: c.timeout,
            max_body: 1 << 20,
        })
        .unwrap(),
    );
    let socket_log = log.log.clone();
    let sockets = Arc::new(SocketServer::new_with_revalidation(
        c.socket_origin,
        socket_authorize,
        socket_revalidate,
        Arc::new(move |admitted: Option<SocketAdmission>| {
            let socket_attribution = if let Some(value) = admitted {
                let admission = Arc::downcast::<WebSocketAdmission>(value).map_err(|_| {
                    Failure::new(Kind::Internal, "invalid socket admission")
                        .with_type("socket.admission_corrupt")
                })?;
                let initiator = p::Actor::new(
                    p::ActorKind::User,
                    admission.principal.principal_id.to_string(),
                )?;
                p::Attribution::new(p::AttributionSpec {
                    initiator: Some(initiator),
                    ..Default::default()
                })?
            } else {
                socket_attribution.clone()
            };
            let connection = socket_factory
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .open(p::RootSpec {
                    work_id: None,
                    origin: p::Origin::Request,
                    operation: p::Operation::new("socket.connection".into())?,
                    attribution: socket_attribution,
                    executor: socket_executor.clone(),
                })?;
            let connection_log = socket_log.with_scope(&connection);
            connection_log.info("socket.connection.opened", BTreeMap::new());
            let f = socket_factory.clone();
            let executor = socket_executor.clone();
            let log = socket_log.clone();
            Ok(SocketSession {
                handle: Arc::new(move |message| {
                    let f = f.clone();
                    let executor = executor.clone();
                    let log = log.clone();
                    let connection = connection.clone();
                    Box::pin(async move {
                        let scope = f.lock().unwrap_or_else(|e| e.into_inner()).child(
                            &connection,
                            p::StepSpec {
                                work_id: None,
                                operation: p::Operation::new("socket.message".into())?,
                                executor,
                                cause: None,
                            },
                        )?;
                        if message.message_type != "ping" {
                            return Err(Failure::new(Kind::Invalid, "unsupported message"));
                        }
                        log.with_scope(&scope)
                            .info("socket.message.completed", BTreeMap::new());
                        Ok(SocketMessage{id:message.id,message_type:"pong".into(),payload:serde_json::json!({"connection_id":connection.snapshot().scope_id.to_string(),"scope_id":scope.snapshot().scope_id.to_string()}).as_object().unwrap().clone()})
                    })
                }),
                close: Box::new(move || {
                    connection_log.info("socket.connection.closed", BTreeMap::new())
                }),
            })
        }),
    ));
    if let Some(runtime) = &audit_runtime {
        tokio.block_on(async { runtime.start() });
    }
    let (code, mut remaining) =
        tokio.block_on(serve(adapter, c.host, c.port, c.shutdown, gate, sockets));
    if let Some(runtime) = audit_runtime {
        let start = Instant::now();
        tokio.block_on(runtime.close(remaining));
        remaining = remaining.saturating_sub(start.elapsed());
    }
    if let Some(client) = outbound_native {
        client.close();
    }
    if let Some(db) = database {
        let start = Instant::now();
        if tokio.block_on(db.close(remaining)).is_err() {
            eprintln!("database shutdown incomplete");
        }
        remaining = remaining.saturating_sub(start.elapsed());
    }
    drop(tokio);
    let started = Instant::now();
    if telemetry.close(remaining).is_err() {
        eprintln!("telemetry shutdown incomplete");
    }
    if log
        .close(remaining.saturating_sub(started.elapsed()))
        .is_err()
    {
        eprintln!("logging shutdown incomplete");
    }
    let events = events.load(Ordering::Relaxed);
    if events > 0 {
        eprintln!("telemetry diagnostic events={events}; exact dropped records unknown");
    }
    code
}
async fn serve(
    adapter: Arc<Server>,
    host: IpAddr,
    port: u16,
    budget: Duration,
    gate: Arc<Gate>,
    sockets: Arc<SocketServer>,
) -> (i32, Duration) {
    let listener = match tokio::net::TcpListener::bind((host, port)).await {
        Ok(l) => l,
        Err(_) => {
            eprintln!("http startup failed");
            return (1, budget);
        }
    };
    gate.start();
    eprintln!("http.listening {}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/_examples/socket",
            axum::routing::get(SocketServer::upgrade),
        )
        .with_state(sockets.clone())
        .merge(
            Router::new()
                .fallback(
                    |State(server): State<Arc<Server>>, req: Request<Body>| async move {
                        server.handle(req).await
                    },
                )
                .with_state(adapter),
        );
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let mut tasks = tokio::task::JoinSet::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(64));
    let mut terminate =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    loop {
        tokio::select! {
         _=tokio::signal::ctrl_c()=>break,
         _=terminate.recv()=>break,
         Some(_)=tasks.join_next()=>{},
         accepted=listener.accept()=>{
          let Ok((stream,_))=accepted else{break};
          let Ok(permit)=slots.clone().try_acquire_owned() else{continue};
          let app=app.clone();let mut shutdown=receiver.clone();
          tasks.spawn(async move{
           let _permit=permit;
           let mut builder=hyper::server::conn::http1::Builder::new();
           builder.timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(1)).max_buf_size(16384);
           let connection=builder.serve_connection(TokioIo::new(stream),TowerToHyperService::new(app)).with_upgrades();
           tokio::pin!(connection);
           tokio::select!{_=connection.as_mut()=>{},_=shutdown.changed()=>{connection.as_mut().graceful_shutdown();let _=connection.await;}}
          });
         }
        }
    }
    gate.drain();
    drop(listener);
    let started = Instant::now();
    if !sockets.close(budget).await {
        eprintln!("socket shutdown incomplete");
    }
    let _ = stop.send(true);
    if tokio::time::timeout(budget.saturating_sub(started.elapsed()), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    (0, budget.saturating_sub(started.elapsed()))
}
