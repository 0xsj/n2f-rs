use n2f_rs::shared::{errors::Classified, keyed::Digest, secret::SecretString};
fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
/// Fixture keys are UTF-8 text (K01/K05); the hex is decoded back to that text.
fn key_text(hex_key: &str) -> String {
    String::from_utf8(hex(hex_key)).expect("fixture keys are UTF-8 text")
}
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../src/shared/keyed/testdata/vectors.json")).unwrap()
}
fn kind_type(e: &n2f_rs::shared::errors::Failure) -> (String, String) {
    let c = e.classification().unwrap();
    (c.kind.as_str().into(), c.error_type.unwrap_or("").into())
}
#[test]
fn k05_vectors_sign_and_verify() {
    let f = fixture();
    for v in f["vectors"].as_array().unwrap() {
        let key = key_text(v["key_hex"].as_str().unwrap());
        let purpose = v["purpose"].as_str().unwrap();
        let message = hex(v["message_hex"].as_str().unwrap());
        let tag = hex(v["tag_hex"].as_str().unwrap());
        let d = Digest::new(SecretString::new(key)).unwrap();
        assert_eq!(
            d.sign(purpose, &message).unwrap().to_vec(),
            tag,
            "{}",
            v["name"]
        );
        assert!(d.verify(purpose, &message, &tag).unwrap());
        let mut wrong = tag.clone();
        wrong[0] ^= 1;
        assert!(!d.verify(purpose, &message, &wrong).unwrap());
        assert!(!d.verify(purpose, &message, &tag[..31]).unwrap());
        assert!(!d.verify("other_purpose", &message, &tag).unwrap());
        let mut other = message.clone();
        other.push(0);
        assert!(!d.verify(purpose, &other, &tag).unwrap());
    }
}
#[test]
fn k02_purpose_binds_and_text_keys_are_utf8_bytes() {
    let f = fixture();
    let v = &f["vectors"][0];
    let d = Digest::new(SecretString::new(key_text(v["key_hex"].as_str().unwrap()))).unwrap();
    let message = hex(v["message_hex"].as_str().unwrap());
    assert_eq!(
        d.sign("csrf", &message).unwrap().to_vec(),
        hex(v["tag_hex"].as_str().unwrap())
    );
    assert_ne!(
        d.sign("csrf", b"m").unwrap(),
        d.sign("rate_limit", b"m").unwrap()
    );
    for r in f["purpose_rejects"].as_array().unwrap() {
        let e = d.sign(r["purpose"].as_str().unwrap(), b"m").unwrap_err();
        assert_eq!(
            kind_type(&e),
            ("invalid".into(), "keyed.purpose_invalid".into()),
            "{}",
            r["name"]
        );
        let e = d
            .verify(r["purpose"].as_str().unwrap(), b"m", &[0u8; 32])
            .unwrap_err();
        assert_eq!(kind_type(&e).1, "keyed.purpose_invalid");
    }
}
#[test]
fn k01_short_or_missing_key_refused_and_key_redacted() {
    let f = fixture();
    let short = key_text(f["short_key_hex"].as_str().unwrap());
    let e = Digest::new(SecretString::new(short)).unwrap_err();
    assert_eq!(
        kind_type(&e),
        ("invalid".into(), "keyed.configuration".into())
    );
    let e = Digest::new(SecretString::new("x".repeat(31))).unwrap_err();
    assert_eq!(kind_type(&e).1, "keyed.configuration");
    let e = Digest::new(SecretString::new(String::new())).unwrap_err();
    assert_eq!(kind_type(&e).1, "keyed.configuration");
    let d = Digest::new(SecretString::new("SENTINEL-key-".repeat(4))).unwrap();
    let shown = format!("{d:?}");
    assert!(!shown.contains("SENTINEL") && shown.contains("REDACTED"));
    assert_eq!(d.sign("csrf", &[]).unwrap().len(), 32);
}
