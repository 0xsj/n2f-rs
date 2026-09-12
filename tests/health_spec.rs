use n2f_rs::shared::health::Gate;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
#[tokio::test]
async fn lifecycle_and_bounded_probe() {
    let g = Gate::new(Duration::from_millis(10), vec![]).unwrap();
    assert!(!g.ready().await);
    assert!(g.start());
    assert!(!g.start());
    assert!(g.ready().await);
    g.drain();
    assert!(!g.ready().await);
    assert!(!g.start());
    let calls = Arc::new(AtomicU32::new(0));
    let count = calls.clone();
    let h = Gate::new(
        Duration::from_millis(10),
        vec![Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::pending())
        })],
    )
    .unwrap();
    h.start();
    assert!(!h.ready().await);
    assert!(!h.ready().await);
    assert_eq!(calls.load(Ordering::SeqCst), 2); // dropped futures leave no detached work
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let e = entered.clone();
    let r = release.clone();
    let d = Arc::new(
        Gate::new(
            Duration::from_secs(1),
            vec![Arc::new(move || {
                let e = e.clone();
                let r = r.clone();
                Box::pin(async move {
                    e.notify_one();
                    r.notified().await;
                    Ok(())
                })
            })],
        )
        .unwrap(),
    );
    d.start();
    let copy = d.clone();
    let task = tokio::spawn(async move { copy.ready().await });
    entered.notified().await;
    d.drain();
    release.notify_one();
    assert!(!task.await.unwrap());
}
