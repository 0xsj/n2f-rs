use n2f_rs::shared::{
    errors::{Classified, Failure, Kind},
    http::{
        CompletionFacts, Outcome, Termination, classify_completion, normalize_method, problem_of,
    },
};
#[test]
fn problem_and_policy() {
    let e = Failure::new(Kind::Conflict, "taken").with_type("account.taken");
    let public = n2f_rs::shared::errors::public_info(e.classification());
    let p = problem_of(Some(&public)).unwrap();
    assert_eq!((p.status, p.code.as_deref()), (409, Some("account.taken")));
    assert_eq!(normalize_method("get"), "_OTHER");
    for (s, o, err) in [
        (200, Outcome::Success, false),
        (409, Outcome::Refused, false),
        (500, Outcome::Failed, true),
    ] {
        let c = classify_completion(CompletionFacts {
            status: Some(s),
            failure_kind: None,
            termination: Termination::ResponseCompleted,
        })
        .unwrap();
        assert_eq!((c.outcome, c.span_error), (o, err));
    }
    let c = classify_completion(CompletionFacts {
        status: None,
        failure_kind: None,
        termination: Termination::PeerClosed,
    })
    .unwrap();
    assert_eq!(c.outcome, Outcome::Canceled);
}
