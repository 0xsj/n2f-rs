use n2f_rs::shared::{
    errors::{Classified, Failure, Kind},
    logger::*,
    secret::SecretString,
};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, UNIX_EPOCH},
};
#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Output {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
    fn records(&self) -> Vec<serde_json::Value> {
        self.text()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
fn config(out: Output, calls: Arc<AtomicUsize>) -> Config {
    Config {
        format: "json".into(),
        level: "info".into(),
        color: "never".into(),
        terminal: false,
        no_color: false,
        resource: Resource {
            name: "test".into(),
            ..Resource::default()
        },
        clock: Arc::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            UNIX_EPOCH + Duration::from_millis(1234)
        }),
        output: Box::new(out),
        capacity: 256,
        max_record_bytes: 65536,
    }
}
fn fields(items: impl IntoIterator<Item = (&'static str, Value)>) -> Fields {
    items.into_iter().map(|(k, v)| (k.into(), v)).collect()
}
#[test]
fn l01_l02_filtering_and_noop() {
    for l in ["debug", "info", "warn", "error"] {
        assert!(parse_level(l).is_ok());
    }
    assert!(parse_level("INFO").is_err());
    for format in ["console", "json", "none"] {
        let out = Output::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut c = config(out.clone(), calls.clone());
        c.format = format.into();
        let r = Runtime::new(c).unwrap();
        r.log.debug("hidden", Fields::new());
        r.log.info("visible", Fields::new());
        r.close(Duration::from_secs(1)).unwrap();
        assert!(!out.text().contains("hidden"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if format == "none" { 0 } else { 1 }
        );
        assert_eq!(
            out.text().lines().count(),
            if format == "none" { 0 } else { 1 }
        );
    }
}
#[test]
fn l03_l10_owned_fields_and_secret() {
    let out = Output::default();
    let r = Runtime::new(config(out.clone(), Arc::default())).unwrap();
    let secret = SecretString::new("credential-SENTINEL".into());
    let mut nested = fields([
        ("token", Value::from(&secret)),
        ("public", "visible".into()),
    ]);
    let parent = r.log.with(fields([
        ("nested", Value::Object(nested.clone())),
        ("keep", 1i64.into()),
    ]));
    let child = parent.with(fields([("keep", 2i64.into()), ("extra", true.into())]));
    nested.insert("public".into(), "changed".into());
    child.info("child", Fields::new());
    parent.info("parent", Fields::new());
    r.close(Duration::from_secs(1)).unwrap();
    let v = out.records();
    assert_eq!(v[0]["timestamp_ms"], 1234);
    assert_eq!(v[0]["level"], "info");
    assert_eq!(v[0]["fields"]["nested"]["public"], "visible");
    assert_eq!(v[0]["fields"]["nested"]["token"], "[REDACTED]");
    assert_eq!(v[0]["fields"]["keep"], 2);
    assert_eq!(v[1]["fields"]["keep"], 1);
    assert!(v[1]["fields"]["extra"].is_null());
    assert!(!out.text().contains("credential-SENTINEL"));
}
#[test]
fn l04_l05_safe_projections() {
    use n2f_rs::shared::{id::Id, provenance as p};
    let out = Output::default();
    let r = Runtime::new(config(out.clone(), Arc::default())).unwrap();
    let mut factory = p::Factory::new(
        || UNIX_EPOCH,
        || Id::parse("01900000-0000-7000-8000-000000000001"),
    );
    let scope = factory
        .open(p::RootSpec {
            origin: p::Origin::Startup,
            operation: p::Operation::new("demo.run".into()).unwrap(),
            attribution: p::Attribution::default(),
            executor: p::Actor::new(p::ActorKind::Service, "api".into()).unwrap(),
            work_id: None,
        })
        .unwrap();
    let log = r.log.with_scope(&scope);
    let unknown = io::Error::other("credential-SENTINEL");
    let e = Failure::new(Kind::Unavailable, "dependency unavailable")
        .with_type("demo.offline")
        .with_source(io::Error::other("credential-SENTINEL"));
    log.with_failure(&e)
        .warn("refused", fields([("scope", "forged".into())]));
    log.with_failure(
        &Failure::new(Kind::Internal, "credential-SENTINEL").with_type("demo.internal"),
    )
    .error("internal", Fields::new());
    log.with_unknown(&unknown).error("unknown", Fields::new());
    r.close(Duration::from_secs(1)).unwrap();
    let v = out.records();
    assert_eq!(
        v[0]["scope"]["scope_id"],
        scope.snapshot().scope_id.to_string()
    );
    assert_eq!(v[0]["error"]["has_cause"], true);
    assert_eq!(v[1]["error"]["type"], "demo.internal");
    assert_eq!(v[2]["error"]["classified"], false);
    assert!(!out.text().contains("credential-SENTINEL"));
}
#[test]
fn l06_color_and_controls() {
    for (mode, tty, no, want) in [
        ("auto", true, false, true),
        ("auto", true, true, false),
        ("auto", false, false, false),
        ("always", false, true, true),
        ("never", true, false, false),
    ] {
        assert_eq!(color_enabled(mode, tty, no).unwrap(), want);
    }
    assert!(color_enabled("bad", false, false).is_err());
    for color in ["always", "never"] {
        let out = Output::default();
        let mut c = config(out.clone(), Arc::default());
        c.format = "console".into();
        c.color = color.into();
        let r = Runtime::new(c).unwrap();
        r.log.info(
            "message\nforged\x1b[31m",
            fields([("field", "value\nnew".into())]),
        );
        r.close(Duration::from_secs(1)).unwrap();
        let s = out.text();
        assert_eq!(s.lines().count(), 1);
        assert!(!s.contains("forged\x1b"));
        assert_eq!(s.contains("\x1b["), color == "always");
    }
}
struct Failed;
impl Write for Failed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("private sink detail"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn l07_sink_failure() {
    let mut c = config(Output::default(), Arc::default());
    c.output = Box::new(Failed);
    let r = Runtime::new(c).unwrap();
    r.log.info("event", Fields::new());
    assert_eq!(
        r.close(Duration::from_secs(1))
            .unwrap_err()
            .classification()
            .unwrap()
            .kind,
        Kind::Unavailable
    );
    assert_eq!(r.stats().failed, 1);
}
struct Blocked {
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Write for Blocked {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let _ = self.started.send(());
        let _ = self.release.recv();
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn l08_queue_and_deadline() {
    let (tx, started) = mpsc::channel();
    let (release, rx) = mpsc::channel();
    let mut c = config(Output::default(), Arc::default());
    c.output = Box::new(Blocked {
        started: tx,
        release: rx,
    });
    c.capacity = 1;
    let r = Runtime::new(c).unwrap();
    r.log.info("first", Fields::new());
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    r.log.info("queued", Fields::new());
    r.log.info("dropped", Fields::new());
    assert_eq!(
        r.close(Duration::from_millis(5))
            .unwrap_err()
            .classification()
            .unwrap()
            .kind,
        Kind::Timeout
    );
    assert_eq!(r.stats().dropped, 1);
    drop(release);
    r.close(Duration::from_secs(1)).unwrap();
}
#[test]
fn l09_invalid_config_time_size_closed() {
    let mut c = config(Output::default(), Arc::default());
    c.format = "bad".into();
    assert!(Runtime::new(c).is_err());
    let mut c = config(Output::default(), Arc::default());
    c.clock = Arc::new(|| UNIX_EPOCH - Duration::from_millis(1));
    let r = Runtime::new(c).unwrap();
    r.log.info("invalid", Fields::new());
    assert_eq!(r.stats().failed, 1);
    assert!(r.close(Duration::from_secs(1)).is_err());
    let out = Output::default();
    let mut c = config(out.clone(), Arc::default());
    c.max_record_bytes = 64;
    let r = Runtime::new(c).unwrap();
    r.log.info(&"x".repeat(100), Fields::new());
    r.close(Duration::from_secs(1)).unwrap();
    r.log.info("closed", Fields::new());
    assert_eq!(r.stats().dropped, 2);
    assert!(out.text().is_empty());
}

#[test]
fn trace_binding_is_owned_and_protected() {
    let out = Output::default();
    let r = Runtime::new(config(out.clone(), Arc::default())).unwrap();
    let trace = n2f_rs::shared::telemetry::TraceRef::parse(
        "12345678901234567890123456789012",
        "1234567890123456",
        false,
    )
    .unwrap();
    r.log
        .with_trace(&trace)
        .with(fields([("trace", "forged".into())]))
        .info("child", Fields::new());
    r.log.info("parent", Fields::new());
    r.close(Duration::from_secs(1)).unwrap();
    let v = out.records();
    assert_eq!(v[0]["trace"]["trace_id"], trace.snapshot().trace_id);
    assert_eq!(v[0]["trace"]["sampled"], false);
    assert!(v[1]["trace"].is_null());
}
