//! Identity PostgreSQL store scenarios (S15) against a disposable real database.
use n2f_rs::{
    domains::identity::{
        app::{
            command::{
                ChallengeReader, ChallengeStore, ChangeRecord, Commit, CredentialReader,
                EpochStore, IssueRecord, LoginRecord, LoginStore, PasswordStore, RegisterOutcome,
                RegisterRecord, RegisterStore, ResetRecord, RevokeOutcome, SessionRevoker,
                VerifyRecord, VerifyStore,
            },
            query::{Resolution, SessionResolver},
        },
        domain::{
            Kind as PrincipalKind, Principal, Status,
            challenge::{Challenge, ChallengeSnapshot},
            credential::CredentialSnapshot,
            email::Email,
            session::{Session, SessionSnapshot},
            token::{TokenDigest, TokenPurpose},
        },
        infra::postgres::{Store, migration},
    },
    shared::{
        errors::{Classified, Failure, Kind},
        events::{Envelope, postgres as events},
        id::Id,
        postgres::{Config, Database, map},
        provenance::{
            Actor, ActorKind, Attribution, Factory, Operation, Origin, RootSpec, WorkContext,
        },
        secret::SecretString,
    },
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

const T0: i64 = 1_700_000_000_000;
const ABS: i64 = 43_200_000;
const IDLE: i64 = 1_800_000;
static NEXT: AtomicU32 = AtomicU32::new(1);

fn id(n: u32) -> Id {
    Id::parse(&format!("01900000-0000-7000-8000-{n:012x}")).unwrap()
}
fn fresh() -> Id {
    id(NEXT.fetch_add(1, Ordering::SeqCst))
}
fn work() -> WorkContext {
    let mut f = Factory::new(
        || UNIX_EPOCH + Duration::from_millis(1000),
        || Ok(id(900_000)),
    );
    f.open(RootSpec {
        work_id: None,
        origin: Origin::Request,
        operation: Operation::new("identity.store.test".into()).unwrap(),
        attribution: Attribution::default(),
        executor: Actor::new(ActorKind::Service, "api".into()).unwrap(),
    })
    .unwrap()
    .work_context()
    .clone()
}
fn event(name: &str, payload: serde_json::Value) -> Envelope {
    Envelope::new(fresh(), name, T0, &work(), payload).unwrap()
}
fn digest(purpose: TokenPurpose, seed: u32) -> TokenDigest {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&seed.to_be_bytes());
    b[31] = 1;
    TokenDigest::parse(purpose, &b).unwrap()
}
fn type_of(e: &Failure) -> String {
    e.classification()
        .unwrap()
        .error_type
        .unwrap_or("")
        .to_string()
}
fn kind_of(e: &Failure) -> Kind {
    e.classification().unwrap().kind
}
async fn scalar(db: &Database, sql: &str) -> i64 {
    let sql = sql.to_owned();
    db.transaction(move |tx| {
        Box::pin(async move { sqlx::query_scalar(&sql).fetch_one(tx).await.map_err(map) })
    })
    .await
    .unwrap()
}
async fn text(db: &Database, sql: &str) -> String {
    let sql = sql.to_owned();
    db.transaction(move |tx| {
        Box::pin(async move { sqlx::query_scalar(&sql).fetch_one(tx).await.map_err(map) })
    })
    .await
    .unwrap()
}
async fn exec(db: &Database, sql: &str) {
    let sql = sql.to_owned();
    db.transaction(move |tx| {
        Box::pin(async move {
            use sqlx::Executor;
            tx.execute(sql.as_str()).await.map_err(map)?;
            Ok(())
        })
    })
    .await
    .unwrap();
}
fn copy_credential(c: &CredentialSnapshot) -> CredentialSnapshot {
    CredentialSnapshot {
        principal_id: c.principal_id,
        email: c.email.clone(),
        password_hash: SecretString::new(c.password_hash.reveal().to_owned()),
        verified_at_ms: c.verified_at_ms,
        password_version: c.password_version,
        created_at_ms: c.created_at_ms,
        changed_at_ms: c.changed_at_ms,
    }
}
struct Registered {
    principal: Id,
    email: Email,
    credential: CredentialSnapshot,
    challenge: ChallengeSnapshot,
}
fn registration(email: &str, seed: u32) -> (RegisterRecord, Registered) {
    let principal = fresh();
    let email = Email::parse(email).unwrap();
    let snapshot = Principal::register(principal, PrincipalKind::Human, "ada".into(), T0)
        .unwrap()
        .snapshot();
    let credential = CredentialSnapshot {
        principal_id: principal,
        email: email.clone(),
        password_hash: SecretString::new("$argon2id$v=19$m=19456,t=2,p=1$hash-SENTINEL".into()),
        verified_at_ms: None,
        password_version: 1,
        created_at_ms: T0,
        changed_at_ms: T0,
    };
    let challenge = Challenge::issue(
        fresh(),
        principal,
        digest(TokenPurpose::EmailVerification, seed),
        1,
        T0,
        T0 + 86_400_000,
    )
    .unwrap()
    .snapshot();
    let ev = event(
        "identity.principal.registered.v1",
        json!({"principal_id": principal.to_string(), "kind": "human", "origin": "self_registration"}),
    );
    let kept = copy_credential(&credential);
    (
        RegisterRecord {
            principal: snapshot,
            auth_epoch: 1,
            credential,
            challenge: challenge.clone(),
            event: ev,
        },
        Registered {
            principal,
            email,
            credential: kept,
            challenge,
        },
    )
}
async fn register(store: &Store, email: &str, seed: u32) -> Registered {
    let (record, r) = registration(email, seed);
    assert_eq!(
        store.register(record).await.unwrap(),
        RegisterOutcome::Created
    );
    r
}
async fn verify(store: &Store, r: &Registered, at: i64) {
    let record = VerifyRecord {
        challenge_id: r.challenge.id,
        principal_id: r.principal,
        consumed_at_ms: at,
        expected_password_version: 1,
        event: event(
            "identity.email.verified.v1",
            json!({"principal_id": r.principal.to_string(), "challenge_id": r.challenge.id.to_string()}),
        ),
    };
    assert_eq!(store.verify_email(record).await.unwrap(), Commit::Committed);
}
fn session(principal: Id, seed: u32, epoch: u32, issued: i64) -> SessionSnapshot {
    Session::issue(
        fresh(),
        principal,
        digest(TokenPurpose::Session, seed),
        epoch,
        issued,
        issued + ABS,
        issued + IDLE,
    )
    .unwrap()
    .snapshot()
}
async fn login(
    store: &Store,
    principal: Id,
    seed: u32,
    epoch: u32,
    issued: i64,
) -> SessionSnapshot {
    let s = session(principal, seed, epoch, issued);
    let record = LoginRecord {
        session: s.clone(),
        expected_password_version: 1,
        expected_auth_epoch: epoch,
        event: event(
            "identity.session.created.v1",
            json!({"principal_id": principal.to_string(), "session_id": s.id.to_string(), "auth_epoch": epoch}),
        ),
    };
    assert_eq!(store.commit_login(record).await.unwrap(), Commit::Committed);
    s
}
async fn open() -> (Arc<Database>, Store) {
    let raw = std::env::var("N2F_TEST_DATABASE_URL").expect("explicit test database required");
    let db = Arc::new(
        Database::open(Config {
            url: SecretString::new(raw),
            max_connections: 16,
            timeout: Duration::from_secs(5),
        })
        .await
        .unwrap(),
    );
    let ledger = vec![events::migration(1), migration(2)];
    db.migrate(ledger.clone()).await.unwrap();
    db.migrate(ledger).await.unwrap();
    (db.clone(), Store::new(db))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires N2F_TEST_DATABASE_URL pointing at a disposable PostgreSQL 18 database"]
async fn identity_store_scenarios() {
    let (db, store) = open().await;
    let store = Arc::new(store);

    // S06 register round trip and exact restoration.
    let ada = register(&store, "Ada.Lovelace@Example.com", 1).await;
    let found = store
        .find_by_email(&ada.email)
        .await
        .unwrap()
        .expect("found");
    assert_eq!(found.principal.id, ada.principal);
    assert_eq!(found.principal.status, Status::Active);
    assert_eq!(found.principal.version, 1);
    assert_eq!(found.auth_epoch, 1);
    assert_eq!(
        found.credential.password_hash.reveal(),
        ada.credential.password_hash.reveal()
    );
    assert_eq!(found.credential.verified_at_ms, None);
    assert_eq!(found.credential.password_version, 1);
    assert_eq!(found.credential.email.reveal(), "ada.lovelace@example.com");
    let by_principal = store
        .find_by_principal(ada.principal)
        .await
        .unwrap()
        .expect("found");
    assert_eq!(by_principal.credential.created_at_ms, T0);
    let ch = store
        .find_by_digest(TokenPurpose::EmailVerification, &ada.challenge.token_digest)
        .await
        .unwrap()
        .expect("challenge");
    assert_eq!(ch.challenge, ada.challenge);
    assert_eq!(ch.challenge.consumed_at_ms, None);
    assert_eq!(ch.challenge.invalidated_at_ms, None);
    assert_eq!(ch.principal_status, Status::Active);
    assert_eq!(ch.auth_epoch, 1);
    assert!(
        store
            .find_by_digest(TokenPurpose::PasswordReset, &ada.challenge.token_digest)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .find_by_email(&Email::parse("nobody@example.com").unwrap())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(scalar(&db, "SELECT count(*) FROM n2f_outbox").await, 1);

    // S06 duplicate leaves nothing behind.
    let before = (
        scalar(&db, "SELECT count(*) FROM n2f_identity_principals").await,
        scalar(&db, "SELECT count(*) FROM n2f_identity_challenges").await,
        scalar(&db, "SELECT count(*) FROM n2f_outbox").await,
    );
    let (dup, _) = registration("ADA.LOVELACE@example.com", 2);
    assert_eq!(
        store.register(dup).await.unwrap(),
        RegisterOutcome::DuplicateEmail
    );
    let after = (
        scalar(&db, "SELECT count(*) FROM n2f_identity_principals").await,
        scalar(&db, "SELECT count(*) FROM n2f_identity_challenges").await,
        scalar(&db, "SELECT count(*) FROM n2f_outbox").await,
    );
    assert_eq!(before, after);
    assert_eq!(text(&db, "SELECT password_hash FROM n2f_identity_credentials WHERE canonical_email='ada.lovelace@example.com'").await, ada.credential.password_hash.reveal());

    // S06 concurrent same-email registration: exactly one created.
    let mut tasks = Vec::new();
    for i in 0..8u32 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            let (record, _) = registration("race@example.com", 100 + i);
            store.register(record).await.unwrap()
        }));
    }
    let mut created = 0;
    for t in tasks {
        if t.await.unwrap() == RegisterOutcome::Created {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM n2f_identity_credentials WHERE canonical_email='race@example.com'"
        )
        .await,
        1
    );

    // S07 commit login: unverified, wrong version, wrong epoch, suspended, then committed.
    let unverified = LoginRecord {
        session: session(ada.principal, 10, 1, T0),
        expected_password_version: 1,
        expected_auth_epoch: 1,
        event: event("identity.session.created.v1", json!({})),
    };
    assert_eq!(store.commit_login(unverified).await.unwrap(), Commit::Stale);
    verify(&store, &ada, T0 + 10).await;
    assert_eq!(text(&db, "SELECT verified_at_ms::text FROM n2f_identity_credentials WHERE canonical_email='ada.lovelace@example.com'").await, (T0 + 10).to_string());
    let stale_version = LoginRecord {
        session: session(ada.principal, 11, 1, T0),
        expected_password_version: 2,
        expected_auth_epoch: 1,
        event: event("identity.session.created.v1", json!({})),
    };
    assert_eq!(
        store.commit_login(stale_version).await.unwrap(),
        Commit::Stale
    );
    let stale_epoch = LoginRecord {
        session: session(ada.principal, 12, 1, T0),
        expected_password_version: 1,
        expected_auth_epoch: 2,
        event: event("identity.session.created.v1", json!({})),
    };
    assert_eq!(
        store.commit_login(stale_epoch).await.unwrap(),
        Commit::Stale
    );
    exec(
        &db,
        &format!(
            "UPDATE n2f_identity_principals SET status='suspended' WHERE id='{}'",
            ada.principal
        ),
    )
    .await;
    let suspended = LoginRecord {
        session: session(ada.principal, 13, 1, T0),
        expected_password_version: 1,
        expected_auth_epoch: 1,
        event: event("identity.session.created.v1", json!({})),
    };
    assert_eq!(store.commit_login(suspended).await.unwrap(), Commit::Stale);
    exec(
        &db,
        &format!(
            "UPDATE n2f_identity_principals SET status='active' WHERE id='{}'",
            ada.principal
        ),
    )
    .await;
    assert_eq!(
        scalar(&db, "SELECT count(*) FROM n2f_identity_sessions").await,
        0
    );
    let outbox_before = scalar(&db, "SELECT count(*) FROM n2f_outbox").await;
    let s1 = login(&store, ada.principal, 20, 1, T0 + 100).await;
    assert_eq!(
        scalar(&db, "SELECT count(*) FROM n2f_outbox").await,
        outbox_before + 1
    );

    // S08 resolve: admitted advances activity; deadline equality rejected and latched.
    match store
        .resolve(&s1.token_digest, T0 + 200, IDLE)
        .await
        .unwrap()
    {
        Resolution::Admitted(p) => {
            assert_eq!(p.principal_id, ada.principal);
            assert_eq!(p.session_id, s1.id);
            assert_eq!(p.auth_epoch, 1);
        }
        other => panic!("expected admitted: {other:?}"),
    }
    let row = format!(
        "SELECT last_seen_at_ms::text || ',' || idle_expires_at_ms::text || ',' || coalesce(revoked_at_ms::text,'none') FROM n2f_identity_sessions WHERE id='{}'",
        s1.id
    );
    assert_eq!(
        text(&db, &row).await,
        format!("{},{},none", T0 + 200, T0 + 200 + IDLE)
    );
    assert!(matches!(
        store
            .resolve(&s1.token_digest, T0 + 150, IDLE)
            .await
            .unwrap(),
        Resolution::Rejected
    ));
    assert_eq!(
        text(&db, &row).await,
        format!("{},{},none", T0 + 200, T0 + 200 + IDLE)
    );
    assert!(matches!(
        store
            .resolve(&s1.token_digest, T0 + 200 + IDLE, IDLE)
            .await
            .unwrap(),
        Resolution::Rejected
    ));
    assert_eq!(
        text(&db, &row).await,
        format!("{},{},{}", T0 + 200, T0 + 200 + IDLE, T0 + 200 + IDLE)
    );
    assert!(matches!(
        store
            .resolve(&digest(TokenPurpose::Session, 999), T0, IDLE)
            .await
            .unwrap(),
        Resolution::Absent
    ));

    // S08/S10 epoch mismatch rejected without a write; S09 revoke observed by resolve.
    let s2 = login(&store, ada.principal, 21, 1, T0 + 300).await;
    let bump = store.revoke_all(ada.principal, 1, T0 + 400, event("identity.sessions.revoked.v1", json!({"principal_id": ada.principal.to_string(), "auth_epoch": 2, "reason": "logout_all"}))).await.unwrap();
    assert_eq!(bump, Commit::Committed);
    assert_eq!(
        store
            .revoke_all(
                ada.principal,
                1,
                T0 + 401,
                event("identity.sessions.revoked.v1", json!({}))
            )
            .await
            .unwrap(),
        Commit::Stale
    );
    assert!(matches!(
        store
            .resolve(&s2.token_digest, T0 + 500, IDLE)
            .await
            .unwrap(),
        Resolution::Rejected
    ));
    assert_eq!(text(&db, &format!("SELECT coalesce(revoked_at_ms::text,'none') || ',' || last_seen_at_ms::text FROM n2f_identity_sessions WHERE id='{}'", s2.id)).await, format!("none,{}", T0 + 300));
    let s3 = login(&store, ada.principal, 22, 2, T0 + 600).await;
    let ev = event(
        "identity.session.revoked.v1",
        json!({"principal_id": ada.principal.to_string(), "session_id": s3.id.to_string(), "reason": "logout"}),
    );
    let outbox_before = scalar(&db, "SELECT count(*) FROM n2f_outbox").await;
    assert_eq!(
        store
            .revoke_session(s3.id, ada.principal, T0 + 650, ev)
            .await
            .unwrap(),
        RevokeOutcome::Revoked
    );
    assert_eq!(
        scalar(&db, "SELECT count(*) FROM n2f_outbox").await,
        outbox_before + 1
    );
    assert_eq!(
        store
            .revoke_session(
                s3.id,
                ada.principal,
                T0 + 660,
                event("identity.session.revoked.v1", json!({}))
            )
            .await
            .unwrap(),
        RevokeOutcome::AlreadyInactive
    );
    assert_eq!(
        scalar(&db, "SELECT count(*) FROM n2f_outbox").await,
        outbox_before + 1
    );
    assert_eq!(
        store
            .revoke_session(
                s3.id,
                fresh(),
                T0 + 660,
                event("identity.session.revoked.v1", json!({}))
            )
            .await
            .unwrap(),
        RevokeOutcome::Absent
    );
    assert!(matches!(
        store
            .resolve(&s3.token_digest, T0 + 700, IDLE)
            .await
            .unwrap(),
        Resolution::Rejected
    ));
    let s4 = login(&store, ada.principal, 23, 2, T0 + 800).await;
    assert_eq!(
        store
            .revoke_session(
                s4.id,
                ada.principal,
                T0 + 800 + ABS + 5,
                event("identity.session.revoked.v1", json!({}))
            )
            .await
            .unwrap(),
        RevokeOutcome::Revoked
    );

    // S11 reissue invalidates outstanding challenges, including expired ones.
    let bob = register(&store, "bob@example.com", 30).await;
    let expired = Challenge::issue(
        fresh(),
        bob.principal,
        digest(TokenPurpose::PasswordReset, 31),
        1,
        T0,
        T0 + 1,
    )
    .unwrap()
    .snapshot();
    assert_eq!(
        store
            .issue_challenge(IssueRecord {
                challenge: expired.clone(),
                expected_password_version: 1
            })
            .await
            .unwrap(),
        Commit::Committed
    );
    let reset = Challenge::issue(
        fresh(),
        bob.principal,
        digest(TokenPurpose::PasswordReset, 32),
        1,
        T0 + 1000,
        T0 + 1000 + 900_000,
    )
    .unwrap()
    .snapshot();
    assert_eq!(
        store
            .issue_challenge(IssueRecord {
                challenge: reset.clone(),
                expected_password_version: 1
            })
            .await
            .unwrap(),
        Commit::Committed
    );
    assert_eq!(
        text(
            &db,
            &format!(
                "SELECT invalidated_at_ms::text FROM n2f_identity_challenges WHERE id='{}'",
                expired.id
            )
        )
        .await,
        (T0 + 1000).to_string()
    );
    assert_eq!(text(&db, &format!("SELECT coalesce(invalidated_at_ms::text,'none') FROM n2f_identity_challenges WHERE id='{}'", bob.challenge.id)).await, "none");
    let stale_issue = Challenge::issue(
        fresh(),
        bob.principal,
        digest(TokenPurpose::PasswordReset, 33),
        1,
        T0 + 2000,
        T0 + 2000 + 900_000,
    )
    .unwrap()
    .snapshot();
    assert_eq!(
        store
            .issue_challenge(IssueRecord {
                challenge: stale_issue,
                expected_password_version: 2
            })
            .await
            .unwrap(),
        Commit::Stale
    );

    // S12 verify with stale version; concurrent consumers of one challenge.
    let stale_verify = VerifyRecord {
        challenge_id: bob.challenge.id,
        principal_id: bob.principal,
        consumed_at_ms: T0 + 5,
        expected_password_version: 2,
        event: event("identity.email.verified.v1", json!({})),
    };
    assert_eq!(
        store.verify_email(stale_verify).await.unwrap(),
        Commit::Stale
    );
    let mut tasks = Vec::new();
    for i in 0..8i64 {
        let store = store.clone();
        let (cid, pid) = (bob.challenge.id, bob.principal);
        tasks.push(tokio::spawn(async move {
            let record = VerifyRecord {
                challenge_id: cid,
                principal_id: pid,
                consumed_at_ms: T0 + 10 + i,
                expected_password_version: 1,
                event: event(
                    "identity.email.verified.v1",
                    json!({"principal_id": pid.to_string(), "challenge_id": cid.to_string()}),
                ),
            };
            store.verify_email(record).await.unwrap()
        }));
    }
    let mut committed = 0;
    for t in tasks {
        if t.await.unwrap() == Commit::Committed {
            committed += 1;
        }
    }
    assert_eq!(committed, 1);
    assert_eq!(scalar(&db, &format!("SELECT count(*) FROM n2f_identity_challenges WHERE id='{}' AND consumed_at_ms IS NOT NULL", bob.challenge.id)).await, 1);
    let consumed_again = VerifyRecord {
        challenge_id: bob.challenge.id,
        principal_id: bob.principal,
        consumed_at_ms: T0 + 50,
        expected_password_version: 1,
        event: event("identity.email.verified.v1", json!({})),
    };
    assert_eq!(
        store.verify_email(consumed_again).await.unwrap(),
        Commit::Stale
    );

    // S13 change password bumps epoch and version, invalidates challenges.
    let bob_session = login(&store, bob.principal, 34, 1, T0 + 3000).await;
    let change = ChangeRecord {
        principal_id: bob.principal,
        new_hash: SecretString::new("$argon2id$new-SENTINEL".into()),
        expected_password_version: 1,
        expected_auth_epoch: 1,
        changed_at_ms: T0 + 4000,
        events: vec![
            event(
                "identity.password.changed.v1",
                json!({"principal_id": bob.principal.to_string(), "password_version": 2, "reason": "change"}),
            ),
            event(
                "identity.sessions.revoked.v1",
                json!({"principal_id": bob.principal.to_string(), "auth_epoch": 2, "reason": "password_change"}),
            ),
        ],
    };
    let outbox_before = scalar(&db, "SELECT count(*) FROM n2f_outbox").await;
    assert_eq!(
        store.change_password(change).await.unwrap(),
        Commit::Committed
    );
    assert_eq!(
        scalar(&db, "SELECT count(*) FROM n2f_outbox").await,
        outbox_before + 2
    );
    let bob_now = store
        .find_by_principal(bob.principal)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bob_now.credential.password_version, 2);
    assert_eq!(bob_now.credential.changed_at_ms, T0 + 4000);
    assert_eq!(
        bob_now.credential.password_hash.reveal(),
        "$argon2id$new-SENTINEL"
    );
    assert_eq!(bob_now.auth_epoch, 2);
    assert!(matches!(
        store
            .resolve(&bob_session.token_digest, T0 + 5000, IDLE)
            .await
            .unwrap(),
        Resolution::Rejected
    ));
    assert_eq!(
        text(
            &db,
            &format!(
                "SELECT invalidated_at_ms::text FROM n2f_identity_challenges WHERE id='{}'",
                reset.id
            )
        )
        .await,
        (T0 + 4000).to_string()
    );
    let stale_change = ChangeRecord {
        principal_id: bob.principal,
        new_hash: SecretString::new("x".into()),
        expected_password_version: 1,
        expected_auth_epoch: 2,
        changed_at_ms: T0 + 4100,
        events: vec![],
    };
    assert_eq!(
        store.change_password(stale_change).await.unwrap(),
        Commit::Stale
    );

    // S14 reset sets verification, consumes the challenge, bumps version and epoch.
    let cy = register(&store, "cy@example.com", 40).await;
    let cy_reset = Challenge::issue(
        fresh(),
        cy.principal,
        digest(TokenPurpose::PasswordReset, 41),
        1,
        T0 + 100,
        T0 + 100 + 900_000,
    )
    .unwrap()
    .snapshot();
    assert_eq!(
        store
            .issue_challenge(IssueRecord {
                challenge: cy_reset.clone(),
                expected_password_version: 1
            })
            .await
            .unwrap(),
        Commit::Committed
    );
    let reset_record = |consumed: i64, version: u32| ResetRecord {
        challenge_id: cy_reset.id,
        principal_id: cy.principal,
        new_hash: SecretString::new("$argon2id$reset-SENTINEL".into()),
        expected_password_version: version,
        expected_auth_epoch: 1,
        consumed_at_ms: consumed,
        set_verified: true,
        events: vec![
            event(
                "identity.password.changed.v1",
                json!({"principal_id": cy.principal.to_string(), "password_version": 2, "reason": "reset"}),
            ),
            event(
                "identity.sessions.revoked.v1",
                json!({"principal_id": cy.principal.to_string(), "auth_epoch": 2, "reason": "password_reset"}),
            ),
        ],
    };
    assert_eq!(
        store
            .reset_password(reset_record(T0 + 100 + 900_000, 1))
            .await
            .unwrap(),
        Commit::Stale
    );
    assert_eq!(
        store
            .reset_password(reset_record(T0 + 200, 2))
            .await
            .unwrap(),
        Commit::Stale
    );
    assert_eq!(
        store
            .reset_password(reset_record(T0 + 200, 1))
            .await
            .unwrap(),
        Commit::Committed
    );
    let cy_now = store
        .find_by_principal(cy.principal)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cy_now.credential.verified_at_ms, Some(T0 + 200));
    assert_eq!(cy_now.credential.password_version, 2);
    assert_eq!(cy_now.auth_epoch, 2);
    assert_eq!(
        text(
            &db,
            &format!(
                "SELECT consumed_at_ms::text FROM n2f_identity_challenges WHERE id='{}'",
                cy_reset.id
            )
        )
        .await,
        (T0 + 200).to_string()
    );
    assert_eq!(
        text(
            &db,
            &format!(
                "SELECT invalidated_at_ms::text FROM n2f_identity_challenges WHERE id='{}'",
                cy.challenge.id
            )
        )
        .await,
        (T0 + 200).to_string()
    );
    assert_eq!(
        store
            .reset_password(reset_record(T0 + 300, 2))
            .await
            .unwrap(),
        Commit::Stale
    );

    // S03 rollback probe: a conflicting outbox row fails every mutation completely.
    async fn poison(db: &Database, e: &Envelope) {
        let poisoned = Envelope::new(
            e.id(),
            "identity.poison.v1",
            T0,
            &work(),
            json!({"poison": true}),
        )
        .unwrap();
        db.transaction(move |tx| Box::pin(async move { events::enqueue(tx, &poisoned).await }))
            .await
            .unwrap();
    }
    let counts = |db: Arc<Database>| async move {
        (
            scalar(&db, "SELECT count(*) FROM n2f_identity_principals").await,
            scalar(&db, "SELECT count(*) FROM n2f_identity_sessions").await,
            scalar(&db, "SELECT count(*) FROM n2f_identity_challenges").await,
            scalar(&db, "SELECT count(*) FROM n2f_outbox").await,
            scalar(&db, "SELECT sum(auth_epoch) FROM n2f_identity_auth_states").await,
            scalar(&db, "SELECT sum(password_version) FROM n2f_identity_credentials").await,
            scalar(&db, "SELECT count(*) FROM n2f_identity_sessions WHERE revoked_at_ms IS NOT NULL").await,
            scalar(&db, "SELECT count(*) FROM n2f_identity_challenges WHERE consumed_at_ms IS NOT NULL OR invalidated_at_ms IS NOT NULL").await,
        )
    };
    let dee = register(&store, "dee@example.com", 50).await;
    verify(&store, &dee, T0 + 10).await;
    let dee_session = login(&store, dee.principal, 51, 1, T0 + 100).await;
    let dee_reset = Challenge::issue(
        fresh(),
        dee.principal,
        digest(TokenPurpose::PasswordReset, 52),
        1,
        T0 + 100,
        T0 + 100 + 900_000,
    )
    .unwrap()
    .snapshot();
    assert_eq!(
        store
            .issue_challenge(IssueRecord {
                challenge: dee_reset.clone(),
                expected_password_version: 1
            })
            .await
            .unwrap(),
        Commit::Committed
    );
    let (probe_record, _) = registration("probe@example.com", 53);
    let baseline = counts(db.clone()).await;
    poison(&db, &probe_record.event).await;
    let expect_reuse = |r: Result<(), Failure>| {
        let e = r.expect_err("enqueue conflict must fail the transaction");
        assert_eq!(type_of(&e), "events.id_reused");
    };
    expect_reuse(store.register(probe_record).await.map(|_| ()));
    let login_ev = event("identity.session.created.v1", json!({}));
    poison(&db, &login_ev).await;
    expect_reuse(
        store
            .commit_login(LoginRecord {
                session: session(dee.principal, 54, 1, T0 + 200),
                expected_password_version: 1,
                expected_auth_epoch: 1,
                event: login_ev,
            })
            .await
            .map(|_| ()),
    );
    let revoke_ev = event("identity.session.revoked.v1", json!({}));
    poison(&db, &revoke_ev).await;
    expect_reuse(
        store
            .revoke_session(dee_session.id, dee.principal, T0 + 300, revoke_ev)
            .await
            .map(|_| ()),
    );
    let all_ev = event("identity.sessions.revoked.v1", json!({}));
    poison(&db, &all_ev).await;
    expect_reuse(
        store
            .revoke_all(dee.principal, 1, T0 + 300, all_ev)
            .await
            .map(|_| ()),
    );
    let verify_ev = event("identity.email.verified.v1", json!({}));
    poison(&db, &verify_ev).await;
    let eve = register(&store, "eve@example.com", 55).await;
    let baseline = {
        let mut b = baseline;
        b.0 += 1;
        b.2 += 1;
        b.3 += 1;
        b.4 += 1;
        b.5 += 1;
        b
    };
    expect_reuse(
        store
            .verify_email(VerifyRecord {
                challenge_id: eve.challenge.id,
                principal_id: eve.principal,
                consumed_at_ms: T0 + 10,
                expected_password_version: 1,
                event: verify_ev,
            })
            .await
            .map(|_| ()),
    );
    let change_ev = event("identity.password.changed.v1", json!({}));
    poison(&db, &change_ev).await;
    expect_reuse(
        store
            .change_password(ChangeRecord {
                principal_id: dee.principal,
                new_hash: SecretString::new("y".into()),
                expected_password_version: 1,
                expected_auth_epoch: 1,
                changed_at_ms: T0 + 400,
                events: vec![event("identity.sessions.revoked.v1", json!({})), change_ev],
            })
            .await
            .map(|_| ()),
    );
    let reset_ev = event("identity.password.changed.v1", json!({}));
    poison(&db, &reset_ev).await;
    expect_reuse(
        store
            .reset_password(ResetRecord {
                challenge_id: dee_reset.id,
                principal_id: dee.principal,
                new_hash: SecretString::new("z".into()),
                expected_password_version: 1,
                expected_auth_epoch: 1,
                consumed_at_ms: T0 + 500,
                set_verified: false,
                events: vec![reset_ev],
            })
            .await
            .map(|_| ()),
    );
    let mut expected = baseline;
    expected.3 += 7; // the poison rows themselves
    assert_eq!(counts(db.clone()).await, expected);
    assert_eq!(
        text(
            &db,
            &format!(
                "SELECT password_hash FROM n2f_identity_credentials WHERE principal_id='{}'",
                dee.principal
            )
        )
        .await,
        dee.credential.password_hash.reveal()
    );
    assert!(matches!(
        store
            .resolve(&dee_session.token_digest, T0 + 600, IDLE)
            .await
            .unwrap(),
        Resolution::Admitted(_)
    ));

    // S15 outbox rows carry no private data.
    assert_eq!(scalar(&db, "SELECT count(*) FROM n2f_outbox WHERE envelope LIKE '%example.com%' OR envelope LIKE '%SENTINEL%'").await, 0);

    // S05 corruption is neither absent nor stale.
    exec(
        &db,
        &format!(
            "UPDATE n2f_identity_principals SET display_name=E'bad\\x01name' WHERE id='{}'",
            eve.principal
        ),
    )
    .await;
    let corrupt = match store.find_by_email(&eve.email).await {
        Err(e) => e,
        Ok(_) => panic!("corrupt row must not restore"),
    };
    assert_eq!(kind_of(&corrupt), Kind::Internal);
    assert_eq!(type_of(&corrupt), "identity.record_corrupt");
    let corrupt_login = store
        .commit_login(LoginRecord {
            session: session(eve.principal, 56, 1, T0 + 700),
            expected_password_version: 1,
            expected_auth_epoch: 1,
            event: event("identity.session.created.v1", json!({})),
        })
        .await
        .expect_err("corrupt row");
    assert_eq!(type_of(&corrupt_login), "identity.record_corrupt");
}
