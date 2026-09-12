use n2f_rs::{
    domains::identity::domain::{Kind, Principal, Snapshot, Status},
    shared::{
        errors::{Classified, Failure},
        id::Id,
    },
};
fn ident() -> Id {
    Id::parse("00000000-0000-4000-8000-000000000001").unwrap()
}
fn code<T>(r: Result<T, Failure>) -> String {
    r.err()
        .unwrap()
        .classification()
        .unwrap()
        .error_type
        .unwrap()
        .to_string()
}
#[test]
fn registration_and_owned_snapshot() {
    for kind in [Kind::Human, Kind::Service] {
        let p = Principal::register(ident(), kind, " Ada 🌱 ".into(), 1000).unwrap();
        let want = Snapshot {
            id: ident(),
            kind,
            display_name: " Ada 🌱 ".into(),
            status: Status::Active,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            version: 1,
        };
        assert_eq!(p.snapshot(), want);
        let mut s = p.snapshot();
        s.display_name = "changed".into();
        assert_eq!(p.snapshot(), want);
    }
}
#[test]
fn strict_name_and_time() {
    for name in [
        "".to_owned(),
        " ".into(),
        "\t".into(),
        "a\n".into(),
        "a\u{7f}".into(),
        "🌱".repeat(101),
    ] {
        assert_eq!(
            code(Principal::register(ident(), Kind::Human, name, 0)),
            "identity.principal_invalid"
        );
    }
    for name in ["a".into(), "🌱".repeat(100), "e\u{301}".into()] {
        assert!(Principal::register(ident(), Kind::Human, name, 0).is_ok());
    }
    for ms in [-1, 253402300800000] {
        assert_eq!(
            code(Principal::register(ident(), Kind::Human, "a".into(), ms)),
            "identity.principal_invalid"
        );
    }
}
#[test]
fn versioned_immutable_lifecycle() {
    let p = Principal::register(ident(), Kind::Human, "Ada".into(), 1000).unwrap();
    let original = p.snapshot();
    let s = p.suspend(1, 1000).unwrap();
    let mut want = original.clone();
    want.status = Status::Suspended;
    want.version = 2;
    assert_eq!(s.snapshot(), want);
    assert_eq!(p.snapshot(), original);
    let a = s.activate(2, 2000).unwrap();
    want.status = Status::Active;
    want.version = 3;
    want.updated_at_ms = 2000;
    assert_eq!(a.snapshot(), want);
    assert_eq!(code(s.suspend(1, 1000)), "identity.version_conflict");
    assert_eq!(code(s.suspend(2, 1000)), "identity.state_conflict");
    assert_eq!(code(s.activate(2, 999)), "identity.principal_invalid");
    let mut full = s.snapshot();
    full.version = 2147483647;
    let full = Principal::restore(full).unwrap();
    assert_eq!(
        code(full.activate(2147483647, 1000)),
        "identity.version_exhausted"
    );
}
#[test]
fn restore_preserves_history() {
    let mut s = Principal::register(ident(), Kind::Human, "Ada".into(), 1000)
        .unwrap()
        .snapshot();
    s.status = Status::Suspended;
    s.version = 8;
    s.updated_at_ms = 2000;
    assert_eq!(Principal::restore(s.clone()).unwrap().snapshot(), s);
    for version in [0, 2147483648] {
        let mut bad = s.clone();
        bad.version = version;
        assert_eq!(code(Principal::restore(bad)), "identity.principal_invalid");
    }
    let mut bad = s.clone();
    bad.updated_at_ms = 999;
    assert!(Principal::restore(bad).is_err());
    let mut bad = s;
    bad.created_at_ms = -1;
    assert!(Principal::restore(bad).is_err());
}
