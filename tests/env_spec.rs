use n2f_rs::shared::{
    env::{Reader, map},
    errors::Classified,
};
use std::collections::BTreeMap;
fn source(items: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    map(items
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}
#[test]
fn v01_presence() {
    let mut r = Reader::new(source(&[("EMPTY", ""), ("SET", "yes")]));
    assert_eq!(r.string("ABSENT", "default"), "default");
    assert_eq!(r.string("EMPTY", "default"), "");
    assert_eq!(r.required("SET"), "yes");
    assert!(r.check().is_ok());
    r.secret("MISSING");
    assert!(r.check().is_err());
}
#[test]
fn v02_parsers() {
    for s in ["", " 2", "2 ", "+2", "02", "1e2", "1.5", "9007199254740992"] {
        let mut r = Reader::new(source(&[("N", s)]));
        r.int("N", 2, -9007199254740991, 9007199254740991);
        assert!(r.check().is_err(), "{s}");
    }
    let mut r = Reader::new(source(&[("N", "-3"), ("B", "false"), ("E", "json")]));
    assert_eq!(r.int("N", 2, -3, 3), -3);
    assert!(!r.boolean("B", true));
    assert_eq!(r.enumeration("E", "console", &["console", "json"]), "json");
    assert!(r.check().is_ok());
    let mut bad = Reader::new(source(&[("B", "TRUE"), ("E", " json")]));
    bad.boolean("B", false);
    bad.enumeration("E", "console", &["console", "json"]);
    assert_eq!(
        bad.check()
            .unwrap_err()
            .classification()
            .unwrap()
            .fields
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn v03_definitions() {
    let mut r =
        Reader::new(|_: &str| -> Option<String> { panic!("invalid definition read source") });
    r.int("N", 20, 0, 10);
    r.enumeration("E", "bad", &["ok"]);
    assert!(r.check().is_err());
}
#[test]
fn v04_failures() {
    let mut r = Reader::new(source(&[("N", "private-SENTINEL"), ("EMPTY", "")]));
    r.int("N", 1, 0, 10);
    r.required("EMPTY");
    let e = r.check().unwrap_err();
    assert_eq!(e.classification().unwrap().fields.unwrap().len(), 2);
    assert!(!format!("{e:?}").contains("private-SENTINEL"));
    assert!(r.manifest().is_err());
}
#[test]
fn v05_manifest() {
    let mut m = BTreeMap::from([
        ("TOKEN".into(), "private-SENTINEL".into()),
        ("NAME".into(), "visible".into()),
    ]);
    let mut r = Reader::new(map(m.clone()));
    m.clear();
    assert_eq!(r.secret("TOKEN").reveal(), "private-SENTINEL");
    r.required("NAME");
    r.int("COUNT", 3, 0, 10);
    let mut v = r.manifest().unwrap();
    assert_eq!(v[0].key, "COUNT");
    assert_eq!(v[0].source, "default");
    assert_eq!(v[1].value, "visible");
    assert_eq!(v[2].value, "[REDACTED]");
    v[2].value = "oops".into();
    assert_eq!(r.manifest().unwrap()[2].value, "[REDACTED]");
}
#[test]
fn v06_duplicate() {
    let mut r = Reader::new(source(&[]));
    r.string("A", "a");
    r.string("A", "b");
    assert_eq!(
        r.check()
            .unwrap_err()
            .classification()
            .unwrap()
            .fields
            .unwrap()["A"],
        "duplicate_key"
    );
}
