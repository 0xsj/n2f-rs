use n2f_rs::shared::clock::{FakeClock, SystemClock};
use std::{
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn c01_initial_and_stable() {
    let c = FakeClock::new(UNIX_EPOCH + Duration::from_nanos(123456789));
    for _ in 0..2 {
        assert_eq!(c.now(), UNIX_EPOCH + Duration::from_nanos(123456789));
        assert_eq!(c.elapsed(), Duration::ZERO);
    }
}
#[test]
fn c02_advance() {
    let c = FakeClock::new(UNIX_EPOCH);
    let d = Duration::from_millis(123) + Duration::from_nanos(7);
    c.advance(d).unwrap();
    assert_eq!(c.now(), UNIX_EPOCH + d);
    assert_eq!(c.elapsed(), d);
    c.advance(Duration::ZERO).unwrap();
    assert_eq!(c.elapsed(), d);
}
#[test]
fn c03_wall_correction() {
    let c = FakeClock::new(UNIX_EPOCH);
    c.advance(Duration::from_secs(1)).unwrap();
    for wall in [
        UNIX_EPOCH + Duration::from_secs(3600),
        UNIX_EPOCH - Duration::from_secs(3600),
    ] {
        c.set(wall);
        assert_eq!(c.now(), wall);
        assert_eq!(c.elapsed(), Duration::from_secs(1));
    }
    c.advance(Duration::from_secs(2)).unwrap();
    assert_eq!(c.now(), UNIX_EPOCH - Duration::from_secs(3598));
    assert_eq!(c.elapsed(), Duration::from_secs(3));
}
#[test]
fn c04_refusal_is_atomic() {
    let c = FakeClock::new(UNIX_EPOCH);
    c.advance(Duration::from_secs(1)).unwrap();
    assert!(c.advance(Duration::MAX).is_err());
    assert_eq!(c.now(), UNIX_EPOCH + Duration::from_secs(1));
    assert_eq!(c.elapsed(), Duration::from_secs(1));
}
#[test]
fn c05_system() {
    let c = SystemClock::new();
    let before = SystemTime::now();
    let wall = c.now();
    let after = SystemTime::now();
    assert!(wall >= before && wall <= after);
    let a = c.elapsed();
    let b = c.elapsed();
    assert!(b >= a);
}
#[test]
fn c06_shared_fake() {
    let c = Arc::new(FakeClock::new(UNIX_EPOCH));
    let workers: Vec<_> = (0..20)
        .map(|_| {
            let c = Arc::clone(&c);
            thread::spawn(move || {
                for _ in 0..100 {
                    c.advance(Duration::from_millis(1)).unwrap();
                    let _ = c.now();
                    let _ = c.elapsed();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(c.elapsed(), Duration::from_secs(2));
    assert_eq!(c.now(), UNIX_EPOCH + Duration::from_secs(2));
}
