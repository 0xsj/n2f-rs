use n2f_rs::shared::secret::SecretString;

#[test]
fn s01_s02_s03_s07_presentation() {
    for raw in ["", " credential-SENTINEL ", "\n\0秘密"] {
        let secret = SecretString::new(raw.to_owned());
        assert_eq!(secret.reveal(), raw);
        assert_eq!(secret.reveal(), raw);
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(format!("{secret:#?}"), "[REDACTED]");
        assert_eq!(format!("{secret:020.2}"), "[REDACTED]");
    }
}
#[test]
fn s04_s05_s06_nested_debug_and_display() {
    #[derive(Debug)]
    struct Record<'a> {
        public: &'a str,
        secrets: &'a [SecretString],
    }
    let values = vec![SecretString::new("credential-SENTINEL".into())];
    let record = Record {
        public: "visible",
        secrets: &values,
    };
    assert_eq!(record.public, "visible");
    assert_eq!(record.secrets.len(), 1);
    let text = format!("{record:#?}");
    assert!(text.contains("visible"));
    assert!(text.contains("[REDACTED]"));
    assert!(!text.contains("credential-SENTINEL"));
}
#[test]
fn s08_s09_owned_value_and_absence() {
    let mut input = "credential-SENTINEL".to_owned();
    let secret = SecretString::new(input.clone());
    input.clear();
    assert_eq!(secret.reveal(), "credential-SENTINEL");
    let empty = Some(SecretString::new(String::new()));
    assert_eq!(empty.as_ref().unwrap().reveal(), "");
    assert_ne!(format!("{empty:?}"), format!("{:?}", None::<SecretString>));
}
