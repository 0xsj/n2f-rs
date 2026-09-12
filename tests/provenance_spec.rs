use n2f_rs::shared::{
    errors::{Classified, Failure, Kind},
    id::Id,
    provenance::*,
};
use std::{
    cell::Cell,
    error::Error,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
fn id(n: u32) -> Id {
    Id::parse(&format!("01900000-0000-7000-8000-{n:012x}")).unwrap()
}
fn actor() -> Actor {
    Actor::new(ActorKind::Service, "api".into()).unwrap()
}
fn op() -> Operation {
    Operation::new("demo.export".into()).unwrap()
}
fn root() -> RootSpec {
    RootSpec {
        work_id: None,
        origin: Origin::Startup,
        operation: op(),
        attribution: Attribution::default(),
        executor: actor(),
    }
}
type F = Factory<Box<dyn Fn() -> SystemTime>, Box<dyn FnMut() -> Result<Id, Failure>>>;
struct Harness {
    f: F,
    time: Rc<Cell<SystemTime>>,
    clock: Rc<Cell<u32>>,
    ids: Rc<Cell<u32>>,
}
fn setup() -> Harness {
    let time = Rc::new(Cell::new(UNIX_EPOCH + Duration::from_millis(1000)));
    let clock = Rc::new(Cell::new(0));
    let ids = Rc::new(Cell::new(0));
    let (t, c, g) = (time.clone(), clock.clone(), ids.clone());
    let f = Factory::new(
        Box::new(move || {
            c.set(c.get() + 1);
            t.get()
        }) as Box<dyn Fn() -> SystemTime>,
        Box::new(move || {
            g.set(g.get() + 1);
            Ok(id(g.get()))
        }) as Box<dyn FnMut() -> Result<Id, Failure>>,
    );
    Harness {
        f,
        time,
        clock,
        ids,
    }
}
fn step() -> StepSpec {
    StepSpec {
        work_id: None,
        operation: op(),
        executor: actor(),
        cause: None,
    }
}
fn kind(e: &Failure, t: &str) {
    assert_eq!(
        e.classification().unwrap().error_type,
        Some(format!("provenance.{t}").as_str())
    );
}
#[test]
fn p01_p02_attribution() {
    for s in ["", "a b", "é"] {
        kind(
            &Actor::new(ActorKind::User, s.into()).unwrap_err(),
            "invalid_actor",
        );
    }
    let a = Actor::new(ActorKind::User, "alice".into()).unwrap();
    let b = Actor::new(ActorKind::User, "bob".into()).unwrap();
    let good = Attribution::new(AttributionSpec {
        initiator: Some(a.clone()),
        on_behalf_of: Some(b.clone()),
        tenant: Some("acme".into()),
    })
    .unwrap();
    assert_eq!(good.snapshot().tenant.as_deref(), Some("acme"));
    for spec in [
        AttributionSpec {
            on_behalf_of: Some(b.clone()),
            ..Default::default()
        },
        AttributionSpec {
            initiator: Some(Actor::anonymous()),
            on_behalf_of: Some(b),
            tenant: None,
        },
        AttributionSpec {
            initiator: Some(a.clone()),
            on_behalf_of: Some(a),
            tenant: None,
        },
    ] {
        kind(&Attribution::new(spec).unwrap_err(), "invalid_attribution");
    }
    assert!(Attribution::default().snapshot().initiator.is_none());
    assert_eq!(Actor::anonymous().kind(), ActorKind::Anonymous);
}
#[test]
fn p03_p04_p05_p06_p07_p08_p09_transitions() {
    let mut h = setup();
    let mut spec = root();
    spec.attribution = Attribution::new(AttributionSpec {
        tenant: Some("acme".into()),
        ..Default::default()
    })
    .unwrap();
    let a = h.f.open(spec).unwrap();
    let s = a.snapshot();
    assert_eq!(
        (s.scope_id, s.work.work_id, s.work.correlation_id),
        (id(1), id(1), id(1))
    );
    assert_eq!((s.attempt, s.work.depth), (1, Some(0)));
    assert_eq!((h.clock.get(), h.ids.get()), (1, 1));
    let child = h.f.child(&a, step()).unwrap().snapshot();
    assert_eq!(child.work.causation.unwrap().id(), s.scope_id);
    assert_eq!(child.work.depth, Some(1));
    assert_eq!(
        child.work.attribution.snapshot().tenant.as_deref(),
        Some("acme")
    );
    let before = h.ids.get();
    let cause = Reference::new(ReferenceKind::Event, id(90)).unwrap();
    let work = prepare(
        &a,
        WorkSpec {
            work_id: id(50),
            operation: op(),
            cause: Some(cause),
        },
    )
    .unwrap();
    assert_eq!((h.clock.get(), h.ids.get()), (before, before));
    let b =
        h.f.execute(
            &work,
            ExecutionSpec {
                executor: actor(),
                attempt: 1,
            },
        )
        .unwrap();
    h.time.set(UNIX_EPOCH + Duration::from_millis(2000));
    let worker = Actor::new(ActorKind::Service, "worker".into()).unwrap();
    let retry = h.f.retry(&b, worker).unwrap().snapshot();
    assert_eq!(retry.work.work_id, id(50));
    assert_eq!(retry.work.causation, Some(cause));
    assert_eq!(retry.previous_attempt, Some(b.snapshot().scope_id));
    assert_eq!(retry.attempt, 2);
    assert_eq!(retry.executor.identity(), "worker");
    assert_eq!(retry.started_at, h.time.get());
    let other = h.f.retry(&b, actor()).unwrap().snapshot();
    assert_eq!(other.attempt, 2);
    assert_ne!(other.scope_id, retry.scope_id);
    let resumed =
        h.f.execute(
            &work,
            ExecutionSpec {
                executor: actor(),
                attempt: 3,
            },
        )
        .unwrap()
        .snapshot();
    assert!(resumed.previous_attempt.is_none());
    let mut explicit = root();
    explicit.work_id = Some(id(80));
    let x = h.f.open(explicit).unwrap().snapshot();
    assert_eq!(x.work.work_id, id(80));
    assert_eq!(x.work.correlation_id, x.scope_id);
}
#[test]
fn p10_p11_bounds_and_wall() {
    let mut h = setup();
    let a = h.f.open(root()).unwrap();
    let mut snap = a.snapshot();
    snap.attempt = u32::MAX;
    let max = restore_scope(snap).unwrap();
    kind(&h.f.retry(&max, actor()).unwrap_err(), "attempt_exhausted");
    assert_eq!((h.clock.get(), h.ids.get()), (1, 1));
    let mut snap = a.snapshot();
    snap.work.depth = Some(u32::MAX);
    snap.work.causation = Some(Reference::new(ReferenceKind::Event, id(90)).unwrap());
    let max = restore_scope(snap).unwrap();
    kind(&h.f.child(&max, step()).unwrap_err(), "depth_exhausted");
    h.time.set(UNIX_EPOCH + Duration::from_millis(1));
    assert!(h.f.child(&a, step()).unwrap().snapshot().started_at < a.snapshot().started_at);
}
#[test]
fn p12_p23_failure_effects() {
    let mut h = setup();
    let mut bad = root();
    bad.executor = Actor::anonymous();
    kind(&h.f.open(bad).unwrap_err(), "invalid_actor");
    assert_eq!((h.clock.get(), h.ids.get()), (0, 0));
    h.time.set(UNIX_EPOCH - Duration::from_millis(1));
    kind(&h.f.open(root()).unwrap_err(), "invalid_time");
    assert_eq!(h.ids.get(), 0);
    let mut f = Factory::new(
        || UNIX_EPOCH,
        || {
            Err(Failure::new(Kind::Unavailable, "entropy")
                .with_type("id.entropy")
                .with_source(std::io::Error::other("source")))
        },
    );
    let e = f.open(root()).unwrap_err();
    assert_eq!(e.classification().unwrap().kind, Kind::Unavailable);
    assert_eq!(e.classification().unwrap().error_type, Some("id.entropy"));
    assert!(
        e.source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .is_some()
    );
    let mut f = Factory::new(|| UNIX_EPOCH, || Ok(id(1)));
    let a = f.open(root()).unwrap();
    kind(&f.child(&a, step()).unwrap_err(), "invalid_generated_id");
}
#[test]
fn p13_links() {
    let target = Reference::new(ReferenceKind::Event, id(90)).unwrap();
    let l = Link {
        relation: Relation::Input,
        target,
    };
    let set = LinkSet::new(vec![l, l]).unwrap();
    assert_eq!(set.values(), &[l]);
    kind(&LinkSet::new(vec![l; 33]).unwrap_err(), "too_many_links");
    kind(
        &LinkSet::new(vec![Link {
            relation: Relation::PreviousAttempt,
            target,
        }])
        .unwrap_err(),
        "invalid_reference",
    );
}
#[test]
fn p14_replay() {
    let mut h = setup();
    let a = h.f.open(root()).unwrap();
    let source = Reference::new(ReferenceKind::Work, a.snapshot().work.work_id).unwrap();
    let r =
        h.f.replay(ReplaySpec {
            source,
            run_id: None,
            work_id: None,
            operation: op(),
            attribution: Attribution::default(),
            executor: actor(),
        })
        .unwrap();
    let s = r.snapshot();
    assert_eq!(s.work.correlation_id, s.scope_id);
    assert_ne!(s.work.work_id, source.id());
    assert_eq!(s.work.replay.as_ref().unwrap().run_id, s.scope_id);
    assert_eq!(s.work.origin, Some(Origin::Replay));
    assert!(s.work.causation.is_none());
    assert_eq!(
        h.f.child(&r, step()).unwrap().snapshot().work.replay,
        s.work.replay
    );
    kind(
        &h.f.replay(ReplaySpec {
            source,
            run_id: None,
            work_id: Some(source.id()),
            operation: op(),
            attribution: Attribution::default(),
            executor: actor(),
        })
        .unwrap_err(),
        "invalid_work",
    );
}
#[test]
fn p15_p16_p17_incoming() {
    let mut h = setup();
    let good = id(80).to_string().to_uppercase();
    let cause = id(90).to_string();
    let cases = [
        (
            IncomingHints {
                correlation: None,
                causation: None,
            },
            Disposition::Fresh,
            0,
            false,
        ),
        (
            IncomingHints {
                correlation: Some(&good),
                causation: None,
            },
            Disposition::Continued,
            0,
            false,
        ),
        (
            IncomingHints {
                correlation: Some(&good),
                causation: Some(IncomingCause {
                    kind: "scope",
                    id: &cause,
                }),
            },
            Disposition::Continued,
            0,
            true,
        ),
        (
            IncomingHints {
                correlation: Some(&good),
                causation: Some(IncomingCause {
                    kind: "scope",
                    id: "private-SENTINEL",
                }),
            },
            Disposition::Continued,
            1,
            false,
        ),
        (
            IncomingHints {
                correlation: Some("bad"),
                causation: Some(IncomingCause {
                    kind: "scope",
                    id: &cause,
                }),
            },
            Disposition::Restarted,
            2,
            false,
        ),
    ];
    for (hints, decision, count, has_cause) in cases {
        let in_ = inspect_incoming(hints);
        assert_eq!(in_.decision(), decision);
        assert_eq!(in_.issues().len(), count);
        assert!(!format!("{:?}", in_.issues()).contains("private-SENTINEL"));
        let s = h.f.enter(root(), &in_).unwrap();
        let v = s.snapshot();
        if decision == Disposition::Continued {
            assert_eq!(v.work.correlation_id, id(80));
            assert!(v.work.origin.is_none() && v.work.depth.is_none());
            assert_eq!(v.work.causation.is_some(), has_cause);
            assert!(
                h.f.child(&s, step())
                    .unwrap()
                    .snapshot()
                    .work
                    .depth
                    .is_none()
            );
        } else {
            assert_eq!(v.work.correlation_id, v.scope_id);
            assert!(v.work.causation.is_none());
        }
    }
}
#[test]
fn p18_p19_snapshots() {
    let mut h = setup();
    let a = h.f.open(root()).unwrap();
    let original = a.snapshot();
    let mut changed = a.snapshot();
    changed.work.depth = Some(9);
    assert_eq!(a.snapshot(), original);
    let mut bad = original.clone();
    bad.attempt = 0;
    kind(&restore_scope(bad).unwrap_err(), "invalid_attempt");
    let mut bad = original.clone();
    bad.previous_attempt = Some(bad.scope_id);
    kind(&restore_scope(bad).unwrap_err(), "invalid_scope");
    let mut bad = original.clone();
    bad.work.replay = Some(ReplayInfo {
        run_id: id(80),
        source: Reference::new(ReferenceKind::Event, id(90)).unwrap(),
    });
    kind(&restore_scope(bad).unwrap_err(), "invalid_origin");
    let mut work = original.work;
    work.causation = Some(Reference::new(ReferenceKind::Work, work.work_id).unwrap());
    assert!(restore_work(work).is_err());
}
