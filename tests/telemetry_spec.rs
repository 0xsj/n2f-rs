use n2f_rs::shared::errors::Classified;
use n2f_rs::shared::telemetry::{Outcome, TraceRef};

#[test]
fn trace_ref_rejects_malformed_and_zero_ids() {
    for (trace, span) in [
        ("", "0000000000000000"),
        ("00000000000000000000000000000000", "0123456789abcdef"),
        ("ABCDEF0123456789ABCDEF0123456789", "0123456789abcdef"),
        ("0123456789abcdef0123456789abcdef", "0123456789abcdeg"),
    ] {
        let error = TraceRef::parse(trace, span, true).expect_err("invalid context accepted");
        assert_eq!(
            error.classification().expect("classification").error_type,
            Some("telemetry.invalid_context")
        );
    }
}

#[test]
fn trace_snapshot_is_owned_and_preserves_sampling() {
    let reference = TraceRef::parse(
        "0123456789abcdef0123456789abcdef",
        "0123456789abcdef",
        false,
    )
    .unwrap();
    let snapshot = reference.snapshot();
    assert_eq!(snapshot.trace_id, "0123456789abcdef0123456789abcdef");
    assert!(!snapshot.sampled);
}

#[test]
fn outcome_is_closed_and_exact() {
    for expected in ["success", "refused", "failed", "canceled", "timed_out"] {
        assert_eq!(Outcome::parse(expected).unwrap().as_str(), expected);
    }
    for invalid in ["", "SUCCESS", "unknown", "timed-out"] {
        let error = Outcome::parse(invalid).expect_err("invalid outcome accepted");
        assert_eq!(
            error.classification().expect("classification").error_type,
            Some("telemetry.invalid_outcome")
        );
    }
}
