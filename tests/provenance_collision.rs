use n2f_rs::shared::{errors::Classified, id::Id, provenance as p};
use std::time::UNIX_EPOCH;
#[test]
fn p23_explicit_cause_cannot_hide_execution_collision() {
    let mut f = p::Factory::new(
        || UNIX_EPOCH,
        || Id::parse("01900000-0000-7000-8000-000000000001"),
    );
    let actor = p::Actor::new(p::ActorKind::Service, "api".into()).unwrap();
    let op = p::Operation::new("demo.run".into()).unwrap();
    let parent = f
        .open(p::RootSpec {
            origin: p::Origin::Startup,
            operation: op.clone(),
            executor: actor.clone(),
            attribution: p::Attribution::default(),
            work_id: None,
        })
        .unwrap();
    let e = f
        .child(
            &parent,
            p::StepSpec {
                operation: op,
                executor: actor,
                work_id: Some(Id::parse("01900000-0000-7000-8000-000000000002").unwrap()),
                cause: Some(
                    p::Reference::new(
                        p::ReferenceKind::Event,
                        Id::parse("01900000-0000-7000-8000-000000000003").unwrap(),
                    )
                    .unwrap(),
                ),
            },
        )
        .unwrap_err();
    assert_eq!(
        e.classification().unwrap().error_type,
        Some("provenance.invalid_generated_id")
    );
}
