use n2f_rs::shared::{
    clock::FakeClock,
    errors::{Classified, Failure, Kind, public_info},
    id::{EntropyError, Id, Sequence, V7},
};
use std::{
    cell::Cell,
    collections::HashSet,
    error::Error,
    io,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, UNIX_EPOCH},
};
const TICK: u64 = 0x0123456789ab;
const FIRST: &str = "01234567-89ab-7000-8000-000000000000";
fn zeros(bytes: &mut [u8]) -> Result<(), EntropyError> {
    bytes.fill(0);
    Ok(())
}
fn failure(e: &Failure, kind: Kind, typ: &str) {
    let c = e.classification().unwrap();
    assert_eq!(c.kind, kind);
    assert_eq!(c.error_type, Some(typ));
}
#[test]
fn i01_parse() {
    for s in [
        "F81D4FAE-7DEC-41D0-A765-00A0C91E6BF6",
        "01234567-89ab-f000-8000-000000000000",
        "01234567-89ab-0000-8000-000000000000",
    ] {
        assert_eq!(Id::parse(s).unwrap().to_string(), s.to_lowercase());
    }
    for s in [
        "".to_owned(),
        format!("{FIRST} "),
        format!(" {FIRST}"),
        FIRST.replace('-', ""),
        format!("{{{FIRST}}}"),
        format!("urn:uuid:{FIRST}"),
        "01234567_89ab-7000-8000-000000000000".into(),
        "01234567-89ab-7000-8000-00000000000g".into(),
        "00000000-0000-0000-0000-000000000000".into(),
        "01234567-89ab-7000-0000-000000000000".into(),
        "01234567-89ab-7000-c000-000000000000".into(),
        "01234567-89ab-7000-f000-000000000000".into(),
        "🙂234567-89ab-7000-8000-000000000000".into(),
    ] {
        let e = Id::parse(&s).unwrap_err();
        failure(&e, Kind::Invalid, "id.invalid");
        assert_eq!(e.to_string(), "invalid ID");
    }
}
#[test]
fn i02_value_and_time() {
    let v: Id = "017F22E2-79B0-7CC3-98C4-DC0C0C07398F".parse().unwrap();
    let set = HashSet::from([v]);
    assert!(set.contains(&Id::parse(&v.to_string()).unwrap()));
    assert_eq!(v.version(), 7);
    assert_eq!(v.unix_millis(), Some(1645557742000));
    assert_eq!(
        Id::parse("f81d4fae-7dec-41d0-a765-00a0c91e6bf6")
            .unwrap()
            .unix_millis(),
        None
    );
}
#[test]
fn i03_exact_v7_bytes() {
    let c = FakeClock::new(UNIX_EPOCH + Duration::from_millis(TICK));
    let mut g = V7::with_entropy(
        || c.now(),
        |b: &mut [u8]| -> Result<(), EntropyError> {
            b.copy_from_slice(&[7, 255, 255, 255, 255, 255, 255, 255, 255, 255]);
            Ok(())
        },
    );
    assert_eq!(
        g.new_id().unwrap().to_string(),
        "01234567-89ab-77ff-bfff-ffffffffffff"
    );
    let mut g = V7::with_entropy(|| c.now(), zeros);
    assert_eq!(g.new_id().unwrap().to_string(), FIRST);
}
#[test]
fn i04_ordering_and_rollback() {
    let c = FakeClock::new(UNIX_EPOCH + Duration::from_millis(TICK));
    let calls = Cell::new(0);
    let mut g = V7::with_entropy(
        || c.now(),
        |b: &mut [u8]| -> Result<(), EntropyError> {
            calls.set(calls.get() + 1);
            b.fill(if calls.get() == 1 { 255 } else { 0 });
            Ok(())
        },
    );
    let a = g.new_id().unwrap();
    let b = g.new_id().unwrap();
    c.set(UNIX_EPOCH + Duration::from_millis(TICK - 100));
    let d = g.new_id().unwrap();
    assert!(a < b && b < d);
    assert_eq!(b.to_string(), "01234567-89ab-7800-8000-000000000000");
    assert_eq!(d.unix_millis(), Some(TICK));
    c.set(UNIX_EPOCH + Duration::from_millis(TICK + 1));
    let e = g.new_id().unwrap();
    assert!(e > d);
    assert_eq!(e.to_string(), "01234567-89ac-7000-8000-000000000000");
}
#[test]
fn i05_exhaustion_and_recovery() {
    let c = FakeClock::new(UNIX_EPOCH + Duration::from_millis(TICK));
    let calls = Cell::new(0);
    let mut g = V7::with_entropy(
        || c.now(),
        |b: &mut [u8]| -> Result<(), EntropyError> {
            calls.set(calls.get() + 1);
            zeros(b)
        },
    );
    let mut last = None;
    for _ in 0..4096 {
        let v = g.new_id().unwrap();
        if let Some(last) = last {
            assert!(v > last);
        }
        last = Some(v);
    }
    for _ in 0..2 {
        failure(&g.new_id().unwrap_err(), Kind::Unavailable, "id.exhausted");
    }
    assert_eq!(calls.get(), 4096);
    c.set(UNIX_EPOCH + Duration::from_millis(TICK - 1));
    failure(&g.new_id().unwrap_err(), Kind::Unavailable, "id.exhausted");
    c.set(UNIX_EPOCH + Duration::from_millis(TICK + 1));
    assert!(g.new_id().unwrap() > last.unwrap());
}
#[test]
fn i06_entropy_failure_atomicity() {
    let c = FakeClock::new(UNIX_EPOCH + Duration::from_millis(TICK));
    let fail = Cell::new(true);
    let mut g = V7::with_entropy(
        || c.now(),
        |b: &mut [u8]| -> Result<(), EntropyError> {
            if fail.get() {
                Err(Box::new(io::Error::other("private entropy diagnostic")))
            } else {
                zeros(b)
            }
        },
    );
    let e = g.new_id().unwrap_err();
    failure(&e, Kind::Unavailable, "id.entropy");
    assert_eq!(
        e.source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .to_string(),
        "private entropy diagnostic"
    );
    assert!(!public_info(e.classification()).message.contains("private"));
    fail.set(false);
    assert_eq!(g.new_id().unwrap().to_string(), FIRST);
    c.set(UNIX_EPOCH + Duration::from_millis(TICK + 100));
    fail.set(true);
    failure(&g.new_id().unwrap_err(), Kind::Unavailable, "id.entropy");
    fail.set(false);
    c.set(UNIX_EPOCH + Duration::from_millis(TICK));
    assert_eq!(
        g.new_id().unwrap().to_string(),
        "01234567-89ab-7001-8000-000000000000"
    );
}
#[test]
fn i07_time_range() {
    let c = FakeClock::new(UNIX_EPOCH);
    let calls = Cell::new(0);
    let mut g = V7::with_entropy(
        || c.now(),
        |b: &mut [u8]| -> Result<(), EntropyError> {
            calls.set(calls.get() + 1);
            zeros(b)
        },
    );
    assert_eq!(
        g.new_id().unwrap().to_string(),
        "00000000-0000-7000-8000-000000000000"
    );
    for wall in [
        UNIX_EPOCH - Duration::from_nanos(1),
        UNIX_EPOCH + Duration::from_millis(1 << 48),
    ] {
        c.set(wall);
        failure(&g.new_id().unwrap_err(), Kind::Invalid, "id.time_range");
    }
    assert_eq!(calls.get(), 1);
    c.set(UNIX_EPOCH + Duration::from_millis((1 << 48) - 1) + Duration::from_nanos(999999));
    assert_eq!(g.new_id().unwrap().unix_millis(), Some((1 << 48) - 1));
}
#[test]
fn i08_sequence() {
    let a = Id::parse(FIRST).unwrap();
    let b = Id::parse("01234567-89ab-7001-8000-000000000000").unwrap();
    let mut items = [a, b];
    let mut s = Sequence::new(&items);
    items[0] = b;
    assert_eq!(s.new_id().unwrap(), a);
    assert_eq!(s.new_id().unwrap(), b);
    for _ in 0..2 {
        failure(
            &s.new_id().unwrap_err(),
            Kind::Unavailable,
            "id.sequence_exhausted",
        );
    }
    failure(
        &Sequence::new(&[]).new_id().unwrap_err(),
        Kind::Unavailable,
        "id.sequence_exhausted",
    );
}
#[test]
fn i09_system_and_shared_generator() {
    let c = Arc::new(FakeClock::new(UNIX_EPOCH + Duration::from_millis(TICK)));
    let mut system = V7::new(|| c.now());
    assert_eq!(system.new_id().unwrap().version(), 7);
    let g = Arc::new(Mutex::new(V7::with_entropy(move || c.now(), zeros)));
    let workers: Vec<_> = (0..10)
        .map(|_| {
            let g = Arc::clone(&g);
            thread::spawn(move || {
                (0..100)
                    .map(|_| g.lock().unwrap().new_id().unwrap())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut seen = HashSet::new();
    for worker in workers {
        for v in worker.join().unwrap() {
            assert!(seen.insert(v));
        }
    }
    assert_eq!(seen.len(), 1000);
}
