use n2f_rs::shared::errors::{
    Classification, Classified, Context, Failure, Kind, PublicInfo, public_info,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

fn fields(key: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(key.into(), value.into())])
}
fn internal() -> PublicInfo {
    PublicInfo {
        kind: Kind::Internal,
        message: "internal error".into(),
        error_type: None,
        fields: BTreeMap::new(),
    }
}

#[test]
fn e01_vocabulary() {
    let cases = [
        (Kind::Internal, "internal"),
        (Kind::Invalid, "invalid"),
        (Kind::NotFound, "not_found"),
        (Kind::Conflict, "conflict"),
        (Kind::Unauthenticated, "unauthenticated"),
        (Kind::Forbidden, "forbidden"),
        (Kind::RateLimited, "rate_limited"),
        (Kind::Unavailable, "unavailable"),
        (Kind::Timeout, "timeout"),
        (Kind::Canceled, "canceled"),
    ];
    assert_eq!(Kind::ALL.len(), cases.len());
    for (kind, name) in cases {
        assert_eq!(Kind::ALL.iter().filter(|entry| **entry == kind).count(), 1);
        assert_eq!(kind.as_str(), name);
        assert_eq!(Kind::parse(name), Some(kind));
    }
    for name in ["", "INTERNAL", "not-found", "other"] {
        assert_eq!(Kind::parse(name), None);
    }
}

#[test]
fn e02_construction() {
    let failure = Failure::new(Kind::Invalid, "email is required")
        .with_type("account.email_required")
        .with_field("email", "required");
    assert_eq!(
        public_info(failure.classification()),
        PublicInfo {
            kind: Kind::Invalid,
            message: "email is required".into(),
            error_type: Some("account.email_required".into()),
            fields: fields("email", "required")
        }
    );
    assert_eq!(
        public_info(Failure::new(Kind::Conflict, "").classification()).message,
        "request failed"
    );
}

#[derive(Debug)]
struct Foreign(&'static str);
impl fmt::Display for Foreign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Foreign {}
impl Classified for Foreign {}

#[test]
fn e03_success_and_unknown_are_different() {
    let success: Result<u32, Failure> = Ok(42);
    let annotated = success.map_err(|e| Context::new("register", e));
    assert!(matches!(annotated, Ok(42)));
    let optional: Result<Option<u32>, Failure> = Ok(None);
    assert!(matches!(
        optional.map_err(|e| Context::new("lookup", e)),
        Ok(None)
    ));
    let foreign = Foreign("PRIVATE");
    assert!(foreign.classification().is_none());
    assert_eq!(public_info(foreign.classification()), internal());
    assert_eq!(
        Failure::new(Kind::Internal, "private")
            .classification()
            .unwrap()
            .kind,
        Kind::Internal
    );
}

#[test]
fn e04_public_projection() {
    let failure = Failure::new(Kind::Internal, "PRIVATE message")
        .with_type("PRIVATE.type")
        .with_field("PRIVATE", "field")
        .with_detail("sql", "PRIVATE SQL")
        .with_source(Foreign("PRIVATE source"));
    assert_eq!(public_info(failure.classification()), internal());
    assert_eq!(failure.details().get("sql").unwrap(), "PRIVATE SQL");
    assert_eq!(failure.source().unwrap().to_string(), "PRIVATE source");
    for kind in Kind::ALL.into_iter().filter(|k| *k != Kind::Internal) {
        let safe = Failure::new(kind, "safe message")
            .with_type("module.condition")
            .with_field("input", "problem");
        let view = public_info(safe.classification());
        assert_eq!(view.kind, kind);
        assert_eq!(view.message, "safe message");
        assert_eq!(view.error_type.as_deref(), Some("module.condition"));
        assert_eq!(view.fields, fields("input", "problem"));
    }
}

#[test]
fn e05_metadata_ownership() {
    let base = Failure::new(Kind::Invalid, "invalid input")
        .with_fields(fields("email", "required"))
        .with_details(fields("operation", "insert"));
    let before = public_info(base.classification());
    let derived = base
        .with_field("email", "malformed")
        .with_detail("operation", "register");
    let mut view = public_info(derived.classification());
    assert_eq!(view.fields, fields("email", "malformed"));
    view.fields.insert("email".into(), "MUTATED".into());
    let mut private = derived.details().clone();
    private.insert("operation".into(), "MUTATED".into());
    assert_eq!(
        public_info(derived.classification()).fields,
        fields("email", "malformed")
    );
    assert_eq!(derived.details(), &fields("operation", "register"));
    assert_eq!(before.fields, fields("email", "required"));
}

#[derive(Debug)]
enum RegisterError {
    EmailTaken(Failure),
}
impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("registration refused")
    }
}
impl Error for RegisterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EmailTaken(e) => Some(e),
        }
    }
}
impl Classified for RegisterError {
    fn classification(&self) -> Option<Classification<'_>> {
        match self {
            Self::EmailTaken(e) => e.classification(),
        }
    }
}

#[test]
fn e06_annotation_keeps_typed_case_and_source() {
    let domain = RegisterError::EmailTaken(
        Failure::new(Kind::Conflict, "email taken")
            .with_type("account.email_taken")
            .with_source(Foreign("PRIVATE source")),
    );
    let context = Context::new("register account", domain);
    assert_eq!(
        public_info(context.classification()).error_type.as_deref(),
        Some("account.email_taken")
    );
    assert!(matches!(context.inner(), RegisterError::EmailTaken(_)));
    assert!(
        context
            .source()
            .unwrap()
            .downcast_ref::<RegisterError>()
            .is_some()
    );
    assert_eq!(
        context
            .source()
            .unwrap()
            .source()
            .unwrap()
            .source()
            .unwrap()
            .to_string(),
        "PRIVATE source"
    );
    assert!(context.to_string().contains("register account"));
    assert!(matches!(context.into_inner(), RegisterError::EmailTaken(_)));
}

#[test]
fn e07_translation_owns_projection_and_keeps_source() {
    let inner = Failure::new(Kind::Conflict, "inner")
        .with_type("inner.type")
        .with_field("inner", "field")
        .with_detail("inner", "detail");
    let outer = Failure::new(Kind::Unavailable, "try later").with_source(inner);
    assert_eq!(
        public_info(outer.classification()),
        PublicInfo {
            kind: Kind::Unavailable,
            message: "try later".into(),
            error_type: None,
            fields: BTreeMap::new()
        }
    );
    assert!(outer.details().is_empty());
    let retained = outer.source().unwrap().downcast_ref::<Failure>().unwrap();
    assert_eq!(
        public_info(retained.classification()).error_type.as_deref(),
        Some("inner.type")
    );
    let replaced = outer.with_source(Foreign("replacement"));
    assert_eq!(replaced.source().unwrap().to_string(), "replacement");
}

#[test]
fn e08_classified_timeouts_and_cancellation_survive_context() {
    for kind in [Kind::Canceled, Kind::Timeout] {
        let e = Context::new("operation", Failure::new(kind, "operation stopped"));
        assert_eq!(e.classification().unwrap().kind, kind);
    }
    assert!(Foreign("context canceled").classification().is_none());
}

#[derive(Debug)]
struct Batch(Vec<Box<dyn Error + Send + Sync>>);
impl fmt::Display for Batch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("independent failures")
    }
}
impl Error for Batch {}
impl Classified for Batch {}

#[test]
fn e09_operation_owned_aggregate_requires_explicit_summary() {
    for reversed in [false, true] {
        let mut items: Vec<Box<dyn Error + Send + Sync>> = vec![
            Box::new(Failure::new(Kind::Unavailable, "known")),
            Box::new(Foreign("PRIVATE unknown")),
        ];
        if reversed {
            items.reverse();
        }
        let batch = Batch(items);
        assert!(batch.classification().is_none());
        assert_eq!(public_info(batch.classification()), internal());
        let summary = Failure::new(Kind::Unavailable, "batch interrupted").with_source(batch);
        assert_eq!(
            public_info(summary.classification()).kind,
            Kind::Unavailable
        );
        assert_eq!(
            summary
                .source()
                .unwrap()
                .downcast_ref::<Batch>()
                .unwrap()
                .0
                .len(),
            2
        );
        assert!(
            summary
                .source()
                .unwrap()
                .downcast_ref::<Batch>()
                .unwrap()
                .0
                .iter()
                .any(|item| item.downcast_ref::<Foreign>().is_some())
        );
    }
}

#[test]
fn e10_merge_and_source_replacement() {
    let base = Failure::new(Kind::Invalid, "invalid input")
        .with_type("account.invalid")
        .with_fields(BTreeMap::from([
            ("email".into(), "required".into()),
            ("name".into(), "required".into()),
        ]))
        .with_details(BTreeMap::from([
            ("operation".into(), "insert".into()),
            ("attempt".into(), "1".into()),
        ]))
        .with_source(Foreign("PRIVATE first"));
    let before = public_info(base.classification());
    let derived = base
        .with_fields(BTreeMap::from([
            ("email".into(), "malformed".into()),
            ("age".into(), "positive".into()),
        ]))
        .with_details(BTreeMap::from([
            ("operation".into(), "register".into()),
            ("worker".into(), "signup".into()),
        ]))
        .with_source(Foreign("PRIVATE second"));
    assert_eq!(
        public_info(derived.classification()),
        PublicInfo {
            kind: Kind::Invalid,
            message: "invalid input".into(),
            error_type: Some("account.invalid".into()),
            fields: BTreeMap::from([
                ("email".into(), "malformed".into()),
                ("name".into(), "required".into()),
                ("age".into(), "positive".into())
            ])
        }
    );
    assert_eq!(
        derived.details(),
        &BTreeMap::from([
            ("operation".into(), "register".into()),
            ("attempt".into(), "1".into()),
            ("worker".into(), "signup".into())
        ])
    );
    assert_eq!(
        derived
            .source()
            .unwrap()
            .downcast_ref::<Foreign>()
            .unwrap()
            .0,
        "PRIVATE second"
    );
    assert_eq!(before.fields.get("email").unwrap(), "required");
    let empty_type = Context::new(
        "register",
        Failure::new(Kind::Conflict, "")
            .with_type("account.old")
            .with_type(""),
    );
    let view = public_info(empty_type.classification());
    assert_eq!(view.error_type, None);
    assert_eq!(view.message, "request failed");
}

#[test]
fn e11_boxed_source_preserves_concrete_diagnostic() {
    let source: Box<dyn std::error::Error + Send + Sync> = Box::new(Foreign("PRIVATE replacement"));
    let base = Failure::new(Kind::Unavailable, "entropy unavailable")
        .with_type("id.entropy")
        .with_field("operation", "generate")
        .with_detail("attempt", "2")
        .with_source(Foreign("PRIVATE first"));
    let before = public_info(base.classification());
    let value = base.with_boxed_source(source);
    assert_eq!(public_info(value.classification()), before);
    assert_eq!(value.details().get("attempt").unwrap(), "2");
    assert_eq!(
        value.source().unwrap().downcast_ref::<Foreign>().unwrap().0,
        "PRIVATE replacement"
    );
}
