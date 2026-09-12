use n2f_rs::{
    domains::identity::domain::{
        auth_state::AuthState,
        challenge::{Challenge, ChallengeSnapshot},
        credential::{CredentialSnapshot, PasswordCredential},
        email::Email,
        password::{NewPassword, PasswordInput},
        session::{Session, SessionSnapshot},
        token::{TokenDigest, TokenPurpose},
    },
    shared::{
        errors::{Classified, Failure, Kind},
        id::Id,
        secret::SecretString,
    },
};
use serde::Deserialize;
fn value<T>(result: Result<T, Failure>) -> T {
    match result {
        Ok(v) => v,
        Err(e) => panic!("expected success: {e}"),
    }
}
fn refusal<T>(result: Result<T, Failure>, kind: Kind, code: &str) {
    let err = match result {
        Ok(_) => panic!("expected refusal {code}"),
        Err(e) => e,
    };
    let c = err.classification().expect("classified error");
    assert_eq!(c.kind, kind);
    assert_eq!(c.error_type, Some(code));
}
fn id() -> Id {
    Id::parse("00000000-0000-4000-8000-000000000002").unwrap()
}
fn principal_id() -> Id {
    Id::parse("00000000-0000-4000-8000-000000000001").unwrap()
}
fn digest(p: TokenPurpose) -> TokenDigest {
    value(TokenDigest::parse(p, &[0; 32]))
}
#[derive(Deserialize)]
struct Fixture {
    group: String,
    name: String,
    input: String,
    value: Option<String>,
    error: Option<String>,
}
#[test]
fn auth_value_fixtures() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../src/domains/identity/domain/testdata/auth_values.json"
    ))
    .unwrap();
    let mut failed = Vec::new();
    for f in fixtures {
        let r: Result<String, Failure> = match f.group.as_str() {
            "email" => Email::parse(&f.input).map(|e| e.reveal().to_owned()),
            "new_password" => NewPassword::parse(SecretString::new(f.input))
                .map(|p| p.secret().reveal().to_owned()),
            "login_password" => PasswordInput::parse(SecretString::new(f.input))
                .map(|p| p.secret().reveal().to_owned()),
            "purpose" => TokenPurpose::parse(&f.input).map(|p| p.as_str().to_owned()),
            _ => panic!("unknown fixture group"),
        };
        let matched = match (r, f.error) {
            (Ok(actual), None) => Some(actual) == f.value,
            (Err(e), Some(code)) => e
                .classification()
                .is_some_and(|c| c.kind == Kind::Invalid && c.error_type == Some(code.as_str())),
            _ => false,
        };
        if !matched {
            failed.push(format!("{}/{}", f.group, f.name));
        }
    }
    assert!(failed.is_empty(), "auth fixture mismatches: {failed:?}");
}
#[test]
fn auth_private_presentation() {
    let email = value(Email::parse("PRIVATE.SENTINEL@example.com"));
    let password = value(NewPassword::parse(SecretString::new(
        "private-password-sentinel".into(),
    )));
    let token = digest(TokenPurpose::Session);
    for out in [
        format!("{email}"),
        format!("{email:?}"),
        format!("{password}"),
        format!("{password:?}"),
        format!("{token}"),
        format!("{token:?}"),
    ] {
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("sentinel"));
    }
}
#[test]
fn auth_digest_owns_bytes() {
    for purpose in [
        TokenPurpose::Session,
        TokenPurpose::EmailVerification,
        TokenPurpose::PasswordReset,
        TokenPurpose::WebSocketUpgrade,
    ] {
        let mut raw = [0; 32];
        raw[0] = 7;
        let d = value(TokenDigest::parse(purpose, &raw));
        raw[0] = 9;
        let mut copy = d.bytes();
        copy[0] = 10;
        assert_eq!(raw[0], 9);
        assert_eq!(copy[0], 10);
        assert_eq!(d.bytes()[0], 7);
        assert_eq!(d.purpose(), purpose);
    }
    for n in [0, 31, 33] {
        refusal(
            TokenDigest::parse(TokenPurpose::Session, &vec![0; n]),
            Kind::Invalid,
            "identity.token_invalid",
        );
    }
}
fn credential_seed() -> CredentialSnapshot {
    CredentialSnapshot {
        principal_id: principal_id(),
        email: value(Email::parse("a@example.com")),
        password_hash: SecretString::new("private-hash-sentinel".into()),
        password_version: 8,
        created_at_ms: 0,
        changed_at_ms: 10,
        verified_at_ms: Some(0),
    }
}
#[test]
fn auth_credential_restore_preserves_presence() {
    let c = value(PasswordCredential::restore(credential_seed()));
    let mut s = c.snapshot();
    assert_eq!(s.password_version, 8);
    assert_eq!(s.changed_at_ms, 10);
    assert_eq!(s.verified_at_ms, Some(0));
    assert_eq!(s.password_hash.reveal(), "private-hash-sentinel");
    s.verified_at_ms = Some(8);
    assert_eq!(c.snapshot().verified_at_ms, Some(0));
    s.verified_at_ms = None;
    assert_eq!(
        value(PasswordCredential::restore(s))
            .snapshot()
            .verified_at_ms,
        None
    );
}
#[test]
fn auth_credential_rejects_corruption() {
    let cases: Vec<fn(&mut CredentialSnapshot)> = vec![
        |s| s.password_version = 0,
        |s| s.password_version = 2147483648,
        |s| s.changed_at_ms = 9,
        |s| s.verified_at_ms = Some(9),
        |s| s.created_at_ms = -1,
        |s| s.changed_at_ms = 253402300800000,
    ];
    for mutate in cases {
        let mut s = credential_seed();
        s.created_at_ms = 10;
        s.verified_at_ms = Some(10);
        mutate(&mut s);
        refusal(
            PasswordCredential::restore(s),
            Kind::Invalid,
            "identity.credential_invalid",
        );
    }
    for hash in ["".to_owned(), "x".repeat(513)] {
        let mut s = credential_seed();
        s.password_hash = SecretString::new(hash);
        refusal(
            PasswordCredential::restore(s),
            Kind::Internal,
            "identity.credential_corrupt",
        );
    }
}
#[test]
fn auth_epoch_guards_and_ownership() {
    let s = value(AuthState::new(principal_id(), 7));
    let next = value(s.invalidate(7));
    assert_eq!(s.epoch(), 7);
    assert_eq!(next.epoch(), 8);
    assert_eq!(next.principal_id(), principal_id());
    refusal(s.invalidate(6), Kind::Conflict, "identity.version_conflict");
    let full = value(AuthState::new(principal_id(), 2147483647));
    refusal(
        full.invalidate(2147483647),
        Kind::Conflict,
        "identity.version_exhausted",
    );
    for epoch in [0, 2147483648] {
        refusal(
            AuthState::new(principal_id(), epoch),
            Kind::Invalid,
            "identity.auth_state_invalid",
        );
    }
}
fn session_seed() -> SessionSnapshot {
    SessionSnapshot {
        id: id(),
        principal_id: principal_id(),
        token_digest: digest(TokenPurpose::Session),
        auth_epoch: 3,
        issued_at_ms: 0,
        last_seen_at_ms: 0,
        absolute_expires_at_ms: 100,
        idle_expires_at_ms: 20,
        revoked_at_ms: None,
    }
}
#[test]
fn auth_session_issue_and_activity() {
    let seed = session_seed();
    let s = value(Session::issue(
        id(),
        principal_id(),
        seed.token_digest,
        3,
        0,
        100,
        20,
    ));
    assert_eq!(s.snapshot(), seed);
    value(s.check(19));
    let next = value(s.touch(10, 30));
    let mut want = seed.clone();
    want.last_seen_at_ms = 10;
    want.idle_expires_at_ms = 40;
    assert_eq!(next.snapshot(), want);
    assert_eq!(s.snapshot(), seed);
    let capped = value(next.touch(30, 90));
    assert_eq!(capped.snapshot().idle_expires_at_ms, 100);
    assert_eq!(capped.snapshot().absolute_expires_at_ms, 100);
    for now in [20, 100] {
        refusal(
            s.check(now),
            Kind::Unauthenticated,
            "identity.session_rejected",
        );
    }
    refusal(
        next.check(9),
        Kind::Unauthenticated,
        "identity.session_rejected",
    );
    refusal(
        s.touch(20, 30),
        Kind::Unauthenticated,
        "identity.session_rejected",
    );
    refusal(s.touch(10, 0), Kind::Invalid, "identity.session_invalid");
    refusal(
        s.touch(10, i64::MAX),
        Kind::Invalid,
        "identity.session_invalid",
    );
}
#[test]
fn auth_session_revocation_and_restore() {
    let s = value(Session::restore(session_seed()));
    let revoked = value(s.revoke(0));
    let again = value(revoked.revoke(10));
    assert_eq!(again.snapshot().revoked_at_ms, Some(0));
    assert_eq!(s.snapshot().revoked_at_ms, None);
    refusal(
        again.check(1),
        Kind::Unauthenticated,
        "identity.session_rejected",
    );
    refusal(
        again.touch(1, 20),
        Kind::Unauthenticated,
        "identity.session_rejected",
    );
    let mut snapshot = again.snapshot();
    let restored = value(Session::restore(snapshot.clone()));
    snapshot.revoked_at_ms = Some(99);
    let mut out = restored.snapshot();
    out.revoked_at_ms = Some(98);
    assert_eq!(restored.snapshot().revoked_at_ms, Some(0));
    value(s.revoke(101));
    refusal(s.revoke(-1), Kind::Invalid, "identity.session_invalid");
}
#[test]
fn auth_session_corrupt_snapshots() {
    let cases: Vec<fn(&mut SessionSnapshot)> = vec![
        |s| s.auth_epoch = 0,
        |s| s.auth_epoch = 2147483648,
        |s| s.issued_at_ms = -1,
        |s| s.last_seen_at_ms = -1,
        |s| s.issued_at_ms = 10,
        |s| s.idle_expires_at_ms = 0,
        |s| s.idle_expires_at_ms = 101,
        |s| s.last_seen_at_ms = 21,
        |s| s.absolute_expires_at_ms = 253402300800000,
        |s| s.revoked_at_ms = Some(-1),
    ];
    for mutate in cases {
        let mut s = session_seed();
        mutate(&mut s);
        refusal(
            Session::restore(s),
            Kind::Invalid,
            "identity.session_invalid",
        );
    }
    let mut s = session_seed();
    s.token_digest = digest(TokenPurpose::PasswordReset);
    refusal(
        Session::restore(s),
        Kind::Invalid,
        "identity.session_invalid",
    );
}
fn challenge_seed(p: TokenPurpose) -> ChallengeSnapshot {
    ChallengeSnapshot {
        id: id(),
        principal_id: principal_id(),
        token_digest: digest(p),
        password_version: 7,
        issued_at_ms: 0,
        expires_at_ms: 100,
        consumed_at_ms: None,
        invalidated_at_ms: None,
    }
}
#[test]
fn auth_challenge_single_use_value() {
    for purpose in [TokenPurpose::EmailVerification, TokenPurpose::PasswordReset] {
        let seed = challenge_seed(purpose);
        let c = value(Challenge::issue(
            id(),
            principal_id(),
            seed.token_digest,
            7,
            0,
            100,
        ));
        assert_eq!(c.snapshot(), seed);
        let used = value(c.consume(purpose, 7, 0));
        let other = if purpose == TokenPurpose::EmailVerification {
            TokenPurpose::PasswordReset
        } else {
            TokenPurpose::EmailVerification
        };
        refusal(
            c.consume(other, 7, 1),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        );
        assert_eq!(
            value(c.consume(purpose, 7, 99)).snapshot().consumed_at_ms,
            Some(99)
        );
        let mut future_seed = seed.clone();
        future_seed.issued_at_ms = 10;
        let future = value(Challenge::restore(future_seed));
        refusal(
            future.consume(purpose, 7, 9),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        );
        assert_eq!(used.snapshot().consumed_at_ms, Some(0));
        assert_eq!(c.snapshot().consumed_at_ms, None);
        refusal(
            used.consume(purpose, 7, 1),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        );
        refusal(
            c.consume(purpose, 6, 1),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        );
        refusal(
            c.consume(TokenPurpose::Session, 7, 1),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        );
        refusal(
            c.consume(purpose, 7, 100),
            Kind::Unauthenticated,
            "identity.challenge_rejected",
        );
        refusal(
            c.consume(purpose, 7, -1),
            Kind::Invalid,
            "identity.challenge_invalid",
        );
        let invalid = value(used.invalidate(10));
        let again = value(invalid.invalidate(20));
        assert_eq!(again.snapshot().consumed_at_ms, Some(0));
        assert_eq!(again.snapshot().invalidated_at_ms, Some(10));
    }
}
#[test]
fn auth_challenge_restore_and_invalidation() {
    let mut seed = challenge_seed(TokenPurpose::PasswordReset);
    seed.invalidated_at_ms = Some(0);
    let c = value(Challenge::restore(seed.clone()));
    seed.invalidated_at_ms = Some(10);
    let mut out = c.snapshot();
    out.invalidated_at_ms = Some(11);
    assert_eq!(c.snapshot().invalidated_at_ms, Some(0));
    refusal(
        c.consume(TokenPurpose::PasswordReset, 7, 0),
        Kind::Unauthenticated,
        "identity.challenge_rejected",
    );
}
#[test]
fn auth_challenge_corrupt_snapshots() {
    let cases: Vec<fn(&mut ChallengeSnapshot)> = vec![
        |s| s.password_version = 0,
        |s| s.password_version = 2147483648,
        |s| s.issued_at_ms = -1,
        |s| s.expires_at_ms = 0,
        |s| s.consumed_at_ms = Some(100),
        |s| s.invalidated_at_ms = Some(-1),
        |s| s.expires_at_ms = 253402300800000,
    ];
    for mutate in cases {
        let mut s = challenge_seed(TokenPurpose::PasswordReset);
        mutate(&mut s);
        refusal(
            Challenge::restore(s),
            Kind::Invalid,
            "identity.challenge_invalid",
        );
    }
    let mut s = challenge_seed(TokenPurpose::Session);
    s.password_version = 7;
    refusal(
        Challenge::restore(s),
        Kind::Invalid,
        "identity.challenge_invalid",
    );
}
