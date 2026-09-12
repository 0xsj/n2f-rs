use n2f_rs::shared::{
    errors::{Classified, Failure, Kind, public_info},
    http::{self, CompletionFacts, Termination},
};
#[test]
fn completion_regressions() {
    for (status, kind, expected) in [
        (503, Kind::Conflict, "503"),
        (500, Kind::Internal, "500"),
        (503, Kind::Unavailable, "503"),
        (200, Kind::Internal, "handler_error"),
        (503, Kind::Timeout, "timeout"),
    ] {
        let c = http::classify_completion(CompletionFacts {
            status: Some(status),
            failure_kind: Some(kind),
            termination: Termination::ResponseCompleted,
        })
        .unwrap();
        assert!(c.span_error, "{kind:?} {status}");
        assert_eq!(c.error_type.as_deref(), Some(expected));
    }
    let c = http::classify_completion(CompletionFacts {
        status: Some(503),
        failure_kind: None,
        termination: Termination::ResponseCompleted,
    })
    .unwrap();
    assert_eq!(c.error_type.as_deref(), Some("503"));
}
#[test]
fn absence_and_unknown_are_distinct() {
    assert!(http::problem_of(None).is_none());
}
#[test]
fn public_error_projection_is_still_available() {
    let e = Failure::new(Kind::Internal, "private").with_type("private.type");
    assert_eq!(public_info(e.classification()).message, "internal error");
}
