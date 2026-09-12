use super::{
    config::{Config, load_config},
    logging::logging,
};
use crate::shared::{
    clock::SystemClock,
    errors::{Classified, Failure, Kind},
    id::{Id, V7},
    logger::{Fields, Logger, Runtime, Value},
    provenance as p,
};
use std::{
    cell::RefCell,
    io::{self, Write},
    sync::Arc,
};
pub fn run(
    lookup: impl Fn(&str) -> Option<String>,
    output: Box<dyn Write + Send>,
    diagnostic: &mut dyn Write,
    terminal: bool,
) -> i32 {
    let mut c = match load_config(lookup) {
        Ok(c) => c,
        Err(e) => {
            let problems = e.classification().and_then(|v| v.fields);
            let _ = writeln!(
                diagnostic,
                "configuration refused: {}",
                serde_json::to_string(&problems).expect("safe string map")
            );
            return 2;
        }
    };
    let mut runtime: Option<Runtime> = None;
    let result = (|| -> Result<(), Failure> {
        let clock = Arc::new(SystemClock::new());
        let mut ids = V7::new(|| clock.now());
        c.resource.instance_id = ids.new_id()?.to_string();
        runtime = Some(logging(&c, clock.clone(), output, terminal)?);
        demo(
            &c,
            &runtime.as_ref().expect("initialized runtime").log,
            &clock,
            || ids.new_id(),
        )
    })();
    if let Some(r) = runtime {
        if r.close(c.flush).is_err() {
            let _ = writeln!(diagnostic, "logging delivery failed");
        }
        if r.stats().dropped > 0 {
            let _ = writeln!(diagnostic, "logging records dropped");
        }
    }
    if result.is_err() {
        let _ = writeln!(diagnostic, "foundations failed");
        1
    } else {
        0
    }
}
fn fields(items: impl IntoIterator<Item = (&'static str, Value)>) -> Fields {
    items.into_iter().map(|(k, v)| (k.into(), v)).collect()
}
fn demo(
    c: &Config,
    log: &Logger,
    clock: &SystemClock,
    generate: impl FnMut() -> Result<Id, Failure>,
) -> Result<(), Failure> {
    let start = clock.elapsed();
    let ids = RefCell::new(generate);
    let mut factory = p::Factory::new(|| clock.now(), || ids.borrow_mut()());
    let executor = p::Actor::new(p::ActorKind::Service, c.resource.name.clone())?;
    let startup = factory.open(p::RootSpec {
        work_id: None,
        origin: p::Origin::Startup,
        operation: p::Operation::new("foundations.startup".into())?,
        attribution: p::Attribution::default(),
        executor: executor.clone(),
    })?;
    let parent = log
        .with_scope(&startup)
        .with(fields([("simulated", true.into())]));
    let manifest = Value::Array(
        c.manifest
            .iter()
            .map(|v| {
                Value::Object(fields([
                    ("key", v.key.clone().into()),
                    ("value", v.value.clone().into()),
                    ("source", v.source.clone().into()),
                    ("secret", v.secret.into()),
                ]))
            })
            .collect(),
    );
    parent.info(
        "foundations.start",
        fields([
            ("config", manifest),
            (
                "credential",
                Value::Object(fields([
                    ("token", Value::from(&c.token)),
                    ("public", "visible".into()),
                ])),
            ),
        ]),
    );
    if c.verbose {
        parent.debug("foundations.debug", fields([("enabled", true.into())]));
    }
    let work_id = ids.borrow_mut()()?;
    let work = p::prepare(
        &startup,
        p::WorkSpec {
            work_id,
            operation: p::Operation::new("demo.export".into())?,
            cause: None,
        },
    )?;
    parent.info(
        "work.prepared",
        fields([("work_id", work_id.to_string().into())]),
    );
    let first = factory.execute(
        &work,
        p::ExecutionSpec {
            executor: executor.clone(),
            attempt: 1,
        },
    )?;
    let unavailable = Failure::new(Kind::Unavailable, "demo dependency unavailable")
        .with_type("demo.offline")
        .with_source(io::Error::other(
            "fixture source retains credential-SENTINEL",
        ));
    log.with_scope(&first)
        .with(fields([("simulated", true.into())]))
        .with_failure(&unavailable)
        .warn("dependency.unavailable", Fields::new());
    // This owner permits one retry of a simulated operation with no external write.
    let second = factory.retry(&first, executor)?;
    log.with_scope(&second)
        .with(fields([("simulated", true.into())]))
        .info(
            "work.completed",
            fields([
                ("result", "exported".into()),
                (
                    "elapsed_ms",
                    Value::Float((clock.elapsed() - start).as_secs_f64() * 1000.0),
                ),
            ]),
        );
    parent
        .with_failure(
            &Failure::new(Kind::Conflict, "demo item already exists").with_type("demo.exists"),
        )
        .warn("operation.conflict", Fields::new());
    parent
        .with_unknown(&io::Error::other(
            "fixture source retains credential-SENTINEL",
        ))
        .error("operation.unknown", Fields::new());
    parent.info(
        "foundations.complete",
        fields([
            ("completed", 1i64.into()),
            ("refused", 1i64.into()),
            ("attempts", 2i64.into()),
            ("unknown", 1i64.into()),
            (
                "elapsed_ms",
                Value::Float((clock.elapsed() - start).as_secs_f64() * 1000.0),
            ),
        ]),
    );
    Ok(())
}
