//! Executable specification for the identity local adapters (infra/local/CONTRACT.md):
//! process-local limiter, file-backed blocklist and undelivered mail.
use n2f_rs::{
    domains::identity::{
        app::command::{Admission, AttemptLimiter, EnrollmentPolicy, MailDelivery},
        domain::{email::Email, password::NewPassword},
        infra::local::{Blocklist, Bucket, Limiter, LimiterConfig, UndeliveredMail},
    },
    shared::{
        errors::{Classified, Failure, Kind},
        keyed::Digest,
        secret::SecretString,
    },
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const NOW: u64 = 1_700_000_000_000;

fn kind_type(e: &Failure) -> (Kind, String) {
    let c = e.classification().unwrap();
    (c.kind, c.error_type.unwrap_or("").into())
}
fn keyed() -> Arc<Digest> {
    Arc::new(Digest::new(SecretString::new("0123456789abcdef0123456789abcdef".into())).unwrap())
}
struct Clock(Arc<Mutex<u64>>);
impl Clock {
    fn new() -> (Self, Arc<Mutex<u64>>) {
        let cell = Arc::new(Mutex::new(NOW));
        (Self(cell.clone()), cell)
    }
    fn source(&self) -> Arc<dyn Fn() -> SystemTime + Send + Sync> {
        let cell = self.0.clone();
        Arc::new(move || UNIX_EPOCH + Duration::from_millis(*cell.lock().unwrap()))
    }
}
fn secret(s: &str) -> SecretString {
    SecretString::new(s.into())
}
fn limiter(config: LimiterConfig) -> (Limiter, Arc<Mutex<u64>>) {
    let (clock, cell) = Clock::new();
    (Limiter::new(keyed(), clock.source(), config).unwrap(), cell)
}

#[tokio::test]
async fn l01_l02_buckets_are_independent_and_refuse_with_retry_after() {
    let mut config = LimiterConfig::defaults();
    config.operations.insert(
        "login".into(),
        (
            Some(Bucket {
                attempts: 2,
                window: Duration::from_secs(60),
            }),
            Some(Bucket {
                attempts: 3,
                window: Duration::from_secs(600),
            }),
        ),
    );
    let (l, cell) = limiter(config);
    // Subject bucket: two attempts permitted, the third refused with the window remainder.
    for _ in 0..2 {
        assert_eq!(
            l.admit("login", &secret("ada@example.com"), "10.0.0.1")
                .await
                .unwrap(),
            Admission::Permitted
        );
    }
    *cell.lock().unwrap() = NOW + 15_000;
    assert_eq!(
        l.admit("login", &secret("ada@example.com"), "10.0.0.1")
            .await
            .unwrap(),
        Admission::Refused {
            retry_after_ms: 45_000
        }
    );
    // A different subject from the same source: the source bucket already holds three
    // attempts (the refused one counted too), so the source limit refuses now.
    assert!(matches!(
        l.admit("login", &secret("bob@example.com"), "10.0.0.1")
            .await
            .unwrap(),
        Admission::Refused { retry_after_ms } if retry_after_ms == 600_000 - 15_000
    ));
    // Same subject, another source: refused by the subject bucket only.
    assert!(matches!(
        l.admit("login", &secret("bob@example.com"), "10.0.0.2")
            .await
            .unwrap(),
        Admission::Permitted
    ));
    // Window expiry resets the subject bucket.
    *cell.lock().unwrap() = NOW + 60_001;
    assert_eq!(
        l.admit("login", &secret("ada@example.com"), "10.0.0.3")
            .await
            .unwrap(),
        Admission::Permitted
    );
    // Operations are keyed separately: register with the same subject is fresh.
    assert_eq!(
        l.admit("register", &secret("ada@example.com"), "10.0.0.4")
            .await
            .unwrap(),
        Admission::Permitted
    );
    // Unknown operation is a dependency failure, never a permit.
    let e = l.admit("unknown_op", &secret("x"), "s").await.unwrap_err();
    assert_eq!(
        kind_type(&e),
        (Kind::Unavailable, "identity.auth_dependency_failed".into())
    );
}

#[tokio::test]
async fn l01_state_holds_only_keyed_digests() {
    let (l, _) = limiter(LimiterConfig::defaults());
    l.admit(
        "login",
        &secret("private-subject-SENTINEL"),
        "source-SENTINEL",
    )
    .await
    .unwrap();
    let state = l.debug_keys();
    assert_eq!(state.len(), 2);
    for key in &state {
        assert_eq!(key.len(), 32);
        assert!(!key.windows(8).any(|w| w == b"SENTINEL"));
    }
}

#[tokio::test]
async fn l03_bounded_store_evicts_expired_and_refuses_when_full() {
    let mut config = LimiterConfig::defaults();
    config.max_keys = 4;
    config.operations.insert(
        "login".into(),
        (
            Some(Bucket {
                attempts: 5,
                window: Duration::from_secs(10),
            }),
            None,
        ),
    );
    let (l, cell) = limiter(config);
    for i in 0..4 {
        l.admit("login", &secret(&format!("s{i}")), "src")
            .await
            .unwrap();
    }
    let e = l.admit("login", &secret("s5"), "src").await.unwrap_err();
    assert_eq!(
        kind_type(&e),
        (Kind::Unavailable, "identity.auth_dependency_failed".into())
    );
    *cell.lock().unwrap() = NOW + 10_001;
    assert_eq!(
        l.admit("login", &secret("s5"), "src").await.unwrap(),
        Admission::Permitted
    );
    assert!(l.debug_keys().len() <= 4);
}

#[test]
fn l02_invalid_limits_refused() {
    let (clock, _) = Clock::new();
    let mut config = LimiterConfig::defaults();
    config.operations.insert(
        "login".into(),
        (
            Some(Bucket {
                attempts: 0,
                window: Duration::from_secs(1),
            }),
            None,
        ),
    );
    let e = Limiter::new(keyed(), clock.source(), config).unwrap_err();
    assert_eq!(
        kind_type(&e),
        (Kind::Invalid, "identity.limiter_configuration".into())
    );
    let mut config = LimiterConfig::defaults();
    config.max_keys = 0;
    let (clock, _) = Clock::new();
    assert!(Limiter::new(keyed(), clock.source(), config).is_err());
}

fn list(contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "n2f-blocklist-{}-{}.txt",
        std::process::id(),
        contents.len()
    ));
    std::fs::write(&path, contents).unwrap();
    path
}
fn password(s: &str) -> NewPassword {
    NewPassword::parse(secret(s)).unwrap()
}

#[tokio::test]
async fn b01_matches_are_exact_after_nfc_and_case_folding() {
    let path = list("# development list\n\nPassword123456789\ncafé au lait au lait\n");
    let b = Blocklist::load(&path).unwrap();
    assert_eq!(b.len(), 2);
    assert!(
        !b.check_blocklist(&password("password123456789"))
            .await
            .unwrap()
    );
    assert!(
        !b.check_blocklist(&password("PASSWORD123456789"))
            .await
            .unwrap()
    );
    // Decomposed é normalizes to the composed entry.
    assert!(
        !b.check_blocklist(&password("cafe\u{0301} au lait au lait"))
            .await
            .unwrap()
    );
    // Substring and superset are allowed: comparison is exact.
    assert!(
        b.check_blocklist(&password("password1234567890"))
            .await
            .unwrap()
    );
    assert!(
        b.check_blocklist(&password("xpassword123456789"))
            .await
            .unwrap()
    );
}

#[test]
fn b02_missing_or_empty_list_is_configuration_failure() {
    let e = Blocklist::load(std::path::Path::new("/nonexistent/blocklist.txt")).unwrap_err();
    assert_eq!(
        kind_type(&e),
        (Kind::Invalid, "identity.blocklist_configuration".into())
    );
    let e = Blocklist::load(&list("# only comments\n\n")).unwrap_err();
    assert_eq!(
        kind_type(&e),
        (Kind::Invalid, "identity.blocklist_configuration".into())
    );
}

#[tokio::test]
async fn m01_undelivered_mail_reports_not_delivered_and_retains_nothing() {
    let mail = UndeliveredMail::default();
    let email = Email::parse("ada@example.com").unwrap();
    assert!(
        !mail
            .send_verification(&email, &secret("token-SENTINEL"), 10)
            .await
    );
    assert!(!mail.send_reset(&email, &secret("token-SENTINEL"), 10).await);
    assert_eq!(mail.counts(), (1, 1));
    let shown = format!("{mail:?}");
    assert!(!shown.contains("SENTINEL") && !shown.contains("ada@"));
}
