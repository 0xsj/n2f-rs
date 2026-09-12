use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use n2f_rs::shared::{pagination as p, validation as v};
#[test]
fn scalars_and_decimals() {
    for s in [
        "",
        "00",
        "01",
        "+1",
        "-1",
        "1.0",
        " 1",
        "1 ",
        "1\n",
        "١",
        "2147483648",
        "12345678901",
    ] {
        assert!(v::decimal(s, 0, 2147483647).is_err(), "{s}");
    }
    for s in ["0", "1", "2147483647"] {
        assert!(v::decimal(s, 0, 2147483647).is_ok());
    }
    for (s, n) in [("😀", 1), ("é", 2), ("\u{a0}", 1)] {
        assert!(v::text(s, n, n, true).is_ok());
    }
    assert!(v::text(" \t\n", 0, 10, true).is_err());
    assert!(v::text("a", 2, 1, false).is_err());
}
#[test]
fn bounded_owned_report() {
    let mut r = v::Report::default();
    assert!(r.result().is_ok());
    for i in 0..32 {
        r.add(&format!("f{i}"), "required").unwrap();
    }
    r.add("f0", "other").unwrap();
    assert!(!r.truncated());
    let mut snapshot = r.issues();
    snapshot[0].code = "mutated".into();
    r.add("overflow", "invalid").unwrap();
    assert!(r.truncated());
    assert_eq!(r.issues().len(), 32);
    assert_eq!(r.issues()[0].code, "required");
    assert!(r.result().is_err());
    assert!(r.add("password", "secret value").is_err());
    assert!(r.add("bad/name", "required").is_err());
}
#[test]
fn cursor_and_limits() {
    assert_eq!(p::size(None).unwrap(), 25);
    for s in ["", "0", "101", "01", "1.1", "1\n"] {
        assert!(p::size(Some(s)).is_err());
    }
    let wire = p::encode("org:1/name", "😀/42").unwrap();
    assert_eq!(p::decode(&wire, "org:1/name").unwrap(), "😀/42");
    assert!(p::decode(&wire, "org:2/name").is_err());
    for raw in [
        r#"[2,"s","p"]"#,
        r#"[1,"s",null]"#,
        r#"[1,"s","\ud800"]"#,
        r#"[1,"s","\udc00"]"#,
        r#"[1,"s","\n"]"#,
        r#"[1,"s",""]"#,
        r#"[1,"s","p",0]"#,
        r#"[1,"s","p"] {}"#,
    ] {
        assert!(p::decode(&B64.encode(raw), "s").is_err(), "{raw}");
    }
    for raw in [
        r#"[1.0,"s","p"]"#,
        r#"[1,"s","\ud83d\ude00"]"#,
        r#"[1,"s","\\ud800"]"#,
    ] {
        assert!(p::decode(&B64.encode(raw), "s").is_ok(), "{raw}");
    }
    for bad in [
        format!("{wire}="),
        format!("{wire}\n"),
        "a".into(),
        "_x".into(),
    ] {
        assert!(p::decode(&bad, "org:1/name").is_err());
    }
}
#[test]
fn bounded_window() {
    let page = p::window(vec!["a", "b", "c"], 2, "s", |s| s.to_string()).unwrap();
    assert!(page.has_more);
    assert_eq!(page.items, vec!["a", "b"]);
    assert_eq!(
        p::decode(page.next_cursor.as_ref().unwrap(), "s").unwrap(),
        "b"
    );
    let empty = p::window(Vec::<String>::new(), 2, "s", Clone::clone).unwrap();
    assert!(!empty.has_more);
    assert!(empty.next_cursor.is_none());
    assert!(p::window(vec![1, 2, 3], 1, "s", ToString::to_string).is_err());
}
