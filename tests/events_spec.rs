use n2f_rs::shared::events::Envelope;
#[test]
fn envelope_owns_full_provenance() {
    let raw = include_bytes!("../src/shared/events/fixtures/envelope.json");
    let event = Envelope::decode(raw).unwrap();
    let again = Envelope::new(
        event.id(),
        "diagnostic.created.v1",
        1000,
        event.work(),
        serde_json::json!({"safe":true}),
    )
    .unwrap();
    assert_eq!(event.work(), again.work());
    for (key, value) in [
        ("v", serde_json::json!(2)),
        ("id", serde_json::json!("invalid")),
        ("type", serde_json::json!("unversioned")),
        ("occurred_at_ms", serde_json::json!(-1)),
        ("payload", serde_json::json!([])),
    ] {
        let mut wire: serde_json::Value = serde_json::from_slice(raw).unwrap();
        wire[key] = value;
        assert!(
            Envelope::decode(&serde_json::to_vec(&wire).unwrap()).is_err(),
            "{key}"
        );
    }
}
