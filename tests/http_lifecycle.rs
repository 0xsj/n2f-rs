use n2f_rs::shared::http::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[test]
fn first_valid_finish_and_independent_outputs() {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let mut a = Active::begin(
        Duration::from_secs(1),
        vec![
            Box::new(|_| panic!("private")),
            Box::new(move |c| {
                assert_eq!(c.elapsed, Duration::from_secs(1));
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        ],
    );
    let facts = CompletionFacts {
        status: Some(200),
        failure_kind: None,
        termination: Termination::ResponseCompleted,
    };
    assert!(a.finish(facts, Duration::ZERO).is_err());
    assert!(a.finish(facts, Duration::from_secs(2)).unwrap());
    assert!(!a.finish(facts, Duration::from_secs(3)).unwrap());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(a.failed_attempts(), 1);
}
