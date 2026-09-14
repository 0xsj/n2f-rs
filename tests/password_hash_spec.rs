use n2f_rs::{
    domains::identity::{
        domain::password::{NewPassword, PasswordInput},
        password_hash::{Hasher, Limits, Outcome, Work, dummy_record},
    },
    shared::{
        entropy::EntropyError,
        errors::{Classified, Failure, Kind},
        secret::SecretString,
    },
};
use serde::Deserialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Deserialize)]
struct HashCase {
    name: String,
    password: String,
    salt_hex: String,
    phc: String,
}
#[derive(Deserialize)]
struct VerifyCase {
    name: String,
    record: String,
    password: String,
    outcome: String,
}
#[derive(Deserialize)]
struct Vectors {
    hashes: Vec<HashCase>,
    dummy_record: String,
    verify: Vec<VerifyCase>,
}
fn vectors() -> Vectors {
    serde_json::from_str(include_str!(
        "../src/domains/identity/password_hash/testdata/vectors.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn refusal<T>(result: Result<T, Failure>, kind: Kind, code: &str) {
    let err = match result {
        Ok(_) => panic!("expected refusal {code}"),
        Err(e) => e,
    };
    let c = err.classification().expect("classified error");
    assert_eq!(c.kind, kind, "{code}");
    assert_eq!(c.error_type, Some(code));
}
fn limits() -> Limits {
    Limits {
        max_concurrent: 2,
        max_queued: 2,
        queue_wait: Duration::from_secs(5),
    }
}
fn fixed(salt: Vec<u8>) -> impl FnMut(&mut [u8]) -> Result<(), EntropyError> + Send {
    move |bytes: &mut [u8]| {
        bytes.copy_from_slice(&salt);
        Ok(())
    }
}
fn failing(bytes: &mut [u8]) -> Result<(), EntropyError> {
    bytes.fill(0);
    Err("entropy unavailable".into())
}
fn login(s: &str) -> PasswordInput {
    PasswordInput::parse(SecretString::new(s.to_owned())).unwrap()
}
fn record(s: &str) -> SecretString {
    SecretString::new(s.to_owned())
}

#[tokio::test]
async fn h01_h10_hashes_reproduce_fixture_records() {
    for case in vectors().hashes {
        let hasher = Hasher::new(fixed(hex(&case.salt_hex)), limits()).unwrap();
        let password = NewPassword::parse(SecretString::new(case.password.clone())).unwrap();
        let hashed = hasher.hash(&password).await.unwrap();
        assert_eq!(hashed.reveal(), case.phc, "{}", case.name);
        assert_eq!(format!("{hashed}"), "[REDACTED]");
        assert_eq!(format!("{hashed:?}"), "[REDACTED]");
        assert_eq!(
            hasher
                .verify(&login(&case.password), &hashed)
                .await
                .unwrap(),
            Outcome::Match
        );
        assert!(!hasher.needs_rehash(&hashed).unwrap());
    }
}

#[tokio::test]
async fn h03_h04_verify_outcomes_follow_fixture() {
    let hasher = Hasher::new(failing, limits()).unwrap();
    for case in vectors().verify {
        let result = hasher
            .verify(&login(&case.password), &record(&case.record))
            .await;
        match case.outcome.as_str() {
            "match" => assert_eq!(result.unwrap(), Outcome::Match, "{}", case.name),
            "mismatch" => assert_eq!(result.unwrap(), Outcome::Mismatch, "{}", case.name),
            "corrupt" => {
                refusal(result, Kind::Internal, "identity.credential_corrupt");
                refusal(
                    hasher.needs_rehash(&record(&case.record)),
                    Kind::Internal,
                    "identity.credential_corrupt",
                );
            }
            other => panic!("unknown outcome {other} in {}", case.name),
        }
    }
}

#[tokio::test]
async fn h03_corrupt_records_never_run_the_work() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let work: Work = Arc::new(move |_pwd: &[u8], _salt: &[u8; 16]| {
        counted.fetch_add(1, Ordering::SeqCst);
        [0u8; 32]
    });
    let hasher = Hasher::with_work(failing, limits(), work).unwrap();
    for case in vectors().verify.iter().filter(|c| c.outcome == "corrupt") {
        let _ = hasher.verify(&login("x"), &record(&case.record)).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn h05_absent_credential_costs_work_and_always_mismatches() {
    let v = vectors();
    let hasher = Hasher::new(failing, limits()).unwrap();
    let dummy_password = v
        .verify
        .iter()
        .find(|c| c.name == "dummy_record_is_real")
        .expect("fixture names the dummy password")
        .password
        .clone();
    assert_eq!(
        hasher
            .verify(&login(&dummy_password), &record(&v.dummy_record))
            .await
            .unwrap(),
        Outcome::Match
    );
    assert_eq!(
        hasher.verify_absent(&login(&dummy_password)).await.unwrap(),
        Outcome::Mismatch
    );
    assert_eq!(
        hasher.verify_absent(&login("anything else")).await.unwrap(),
        Outcome::Mismatch
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let work: Work = Arc::new(move |_: &[u8], _: &[u8; 16]| {
        counted.fetch_add(1, Ordering::SeqCst);
        [0u8; 32]
    });
    let seamed = Hasher::with_work(failing, limits(), work).unwrap();
    assert_eq!(
        seamed.verify_absent(&login("anything")).await.unwrap(),
        Outcome::Mismatch
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // The adapter works against exactly the fixture's dummy record: a seam that
    // returns that record's own tag from its own salt must still report Mismatch.
    assert_eq!(dummy_record(), v.dummy_record);
    let fields: Vec<&str> = v.dummy_record.trim_start_matches('$').split('$').collect();
    let salt = std_b64(fields[3]);
    let tag = std_b64(fields[4]);
    let expected_salt: [u8; 16] = salt.try_into().unwrap();
    let expected_tag: [u8; 32] = tag.try_into().unwrap();
    let echo: Work = Arc::new(move |_: &[u8], salt: &[u8; 16]| {
        assert_eq!(*salt, expected_salt, "dummy salt in use");
        expected_tag
    });
    let echoing = Hasher::with_work(failing, limits(), echo).unwrap();
    assert_eq!(
        echoing.verify_absent(&login("anything")).await.unwrap(),
        Outcome::Mismatch
    );
}
fn std_b64(field: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = Vec::new();
    for c in field.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c).unwrap() as u32;
        bits = (bits << 6) | v;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

#[tokio::test]
async fn h02_entropy_failure_refuses_without_fallback() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let work: Work = Arc::new(move |_: &[u8], _: &[u8; 16]| {
        counted.fetch_add(1, Ordering::SeqCst);
        [7u8; 32]
    });
    let hasher = Hasher::with_work(failing, limits(), work).unwrap();
    let password =
        NewPassword::parse(SecretString::new("correct horse battery staple".into())).unwrap();
    refusal(
        hasher.hash(&password).await,
        Kind::Unavailable,
        "identity.entropy_unavailable",
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "no hash with a fallback salt"
    );
}

#[test]
fn h08_configuration_is_validated() {
    for bad in [
        Limits {
            max_concurrent: 0,
            max_queued: 1,
            queue_wait: Duration::from_millis(1),
        },
        Limits {
            max_concurrent: 1,
            max_queued: 0,
            queue_wait: Duration::ZERO,
        },
    ] {
        refusal(
            Hasher::new(failing, bad),
            Kind::Invalid,
            "identity.hasher_configuration",
        );
    }
    assert!(
        Hasher::new(
            failing,
            Limits {
                max_concurrent: 1,
                max_queued: 0,
                queue_wait: Duration::from_millis(1)
            }
        )
        .is_ok()
    );
}

struct Gate {
    release: mpsc::Receiver<()>,
}
fn blocking_work(started: mpsc::Sender<()>, gate: Gate) -> Work {
    let gate = std::sync::Mutex::new(gate);
    Arc::new(move |_: &[u8], _: &[u8; 16]| {
        started.send(()).unwrap();
        gate.lock().unwrap().release.recv().unwrap();
        [1u8; 32]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h06_saturation_and_queue_timeout_refuse_without_a_slot() {
    let (started, on_start) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let hasher = Arc::new(
        Hasher::with_work(
            failing,
            Limits {
                max_concurrent: 1,
                max_queued: 1,
                queue_wait: Duration::from_millis(50),
            },
            blocking_work(started, Gate { release: gate }),
        )
        .unwrap(),
    );
    let input = login("correct horse battery staple");
    let rec = record(&vectors().hashes[0].phc);
    let first = {
        let h = hasher.clone();
        let (i, r) = (login("correct horse battery staple"), record(rec.reveal()));
        tokio::spawn(async move { h.verify(&i, &r).await })
    };
    on_start.recv_timeout(Duration::from_secs(2)).unwrap();
    // One slot busy: the next caller queues and times out; a third finds the queue full.
    let queued = {
        let h = hasher.clone();
        let (i, r) = (login("correct horse battery staple"), record(rec.reveal()));
        tokio::spawn(async move { h.verify(&i, &r).await })
    };
    tokio::time::sleep(Duration::from_millis(5)).await;
    refusal(
        hasher.verify(&input, &rec).await,
        Kind::Unavailable,
        "identity.hash_saturated",
    );
    refusal(
        queued.await.unwrap(),
        Kind::Timeout,
        "identity.hash_queue_timeout",
    );
    // The timed-out waiter left the queue: a new caller can queue again.
    let again = {
        let h = hasher.clone();
        let (i, r) = (login("correct horse battery staple"), record(rec.reveal()));
        tokio::spawn(async move { h.verify(&i, &r).await })
    };
    tokio::time::sleep(Duration::from_millis(5)).await;
    release.send(()).unwrap();
    assert_eq!(first.await.unwrap().unwrap(), Outcome::Mismatch);
    on_start.recv_timeout(Duration::from_secs(2)).unwrap();
    release.send(()).unwrap();
    assert_eq!(again.await.unwrap().unwrap(), Outcome::Mismatch);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h06_abandoned_waiter_leaves_the_queue() {
    let (started, on_start) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let hasher = Arc::new(
        Hasher::with_work(
            failing,
            Limits {
                max_concurrent: 1,
                max_queued: 1,
                queue_wait: Duration::from_secs(5),
            },
            blocking_work(started, Gate { release: gate }),
        )
        .unwrap(),
    );
    let rec = record(&vectors().hashes[0].phc);
    let h = hasher.clone();
    let r = record(rec.reveal());
    let first = tokio::spawn(async move { h.verify(&login("a"), &r).await });
    on_start.recv_timeout(Duration::from_secs(2)).unwrap();
    let h = hasher.clone();
    let r = record(rec.reveal());
    let abandoned = tokio::spawn(async move { h.verify(&login("b"), &r).await });
    tokio::time::sleep(Duration::from_millis(5)).await;
    abandoned.abort();
    let _ = abandoned.await;
    // The abandoned waiter no longer occupies the queue.
    let h = hasher.clone();
    let r = record(rec.reveal());
    let waiting = tokio::spawn(async move { h.verify(&login("c"), &r).await });
    tokio::time::sleep(Duration::from_millis(5)).await;
    refusal(
        hasher.verify(&login("d"), &rec).await,
        Kind::Unavailable,
        "identity.hash_saturated",
    );
    release.send(()).unwrap();
    first.await.unwrap().unwrap();
    on_start.recv_timeout(Duration::from_secs(2)).unwrap();
    release.send(()).unwrap();
    waiting.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h07_in_flight_work_completes_past_the_queue_wait_and_releases_after_panic() {
    let slow: Work = Arc::new(|_: &[u8], _: &[u8; 16]| {
        std::thread::sleep(Duration::from_millis(60));
        [0u8; 32]
    });
    let hasher = Hasher::with_work(
        failing,
        Limits {
            max_concurrent: 1,
            max_queued: 0,
            queue_wait: Duration::from_millis(10),
        },
        slow,
    )
    .unwrap();
    let rec = record(&vectors().hashes[0].phc);
    assert_eq!(
        hasher.verify(&login("a"), &rec).await.unwrap(),
        Outcome::Mismatch,
        "queue wait does not bound admitted work"
    );
    let panicking: Work = Arc::new(|_: &[u8], _: &[u8; 16]| panic!("work failed"));
    let hasher = Hasher::with_work(
        failing,
        Limits {
            max_concurrent: 1,
            max_queued: 0,
            queue_wait: Duration::from_millis(10),
        },
        panicking,
    )
    .unwrap();
    refusal(
        hasher.verify(&login("a"), &rec).await,
        Kind::Internal,
        "identity.hash_failed",
    );
    // The slot was released: the next call is admitted, not saturated.
    refusal(
        hasher.verify(&login("a"), &rec).await,
        Kind::Internal,
        "identity.hash_failed",
    );
}

#[test]
fn h09_failures_carry_no_secret_text() {
    let err = Hasher::new(
        failing,
        Limits {
            max_concurrent: 0,
            max_queued: 0,
            queue_wait: Duration::ZERO,
        },
    )
    .err()
    .unwrap();
    assert!(!format!("{err} {err:?}").contains("correct horse"));
}
