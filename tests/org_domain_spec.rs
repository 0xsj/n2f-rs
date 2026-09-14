use n2f_rs::domains::org::domain::{
    Membership, MembershipSnapshot, Organization, OrganizationSnapshot, Role,
};
use n2f_rs::shared::{
    errors::{Classified, Failure},
    id::Id,
};

fn org_id() -> Id {
    Id::parse("00000000-0000-4000-8000-000000000010").unwrap()
}
fn principal_id() -> Id {
    Id::parse("00000000-0000-4000-8000-000000000011").unwrap()
}
fn membership_id() -> Id {
    Id::parse("00000000-0000-4000-8000-000000000012").unwrap()
}
fn code<T>(result: Result<T, Failure>) -> String {
    result
        .err()
        .unwrap()
        .classification()
        .unwrap()
        .error_type
        .unwrap()
        .to_string()
}

#[test]
fn organization_preserves_value_and_snapshot_ownership() {
    let organization = Organization::create(org_id(), " Team 🌱 ".into(), 1000).unwrap();
    let want = OrganizationSnapshot {
        id: org_id(),
        name: " Team 🌱 ".into(),
        created_at_ms: 1000,
    };
    assert_eq!(organization.snapshot(), want);
    let mut snapshot = organization.snapshot();
    snapshot.name = "changed".into();
    assert_eq!(organization.snapshot(), want);
}

#[test]
fn organization_rejects_invalid_data() {
    for name in ["", " ", "\t", "a\n", "a\u{7f}", "🌱".repeat(101).as_str()] {
        assert_eq!(
            code(Organization::create(org_id(), name.into(), 0)),
            "org.organization_invalid"
        );
    }
    for name in ["a", "🌱🌱", "e\u{301}"] {
        assert!(Organization::create(org_id(), name.into(), 0).is_ok());
    }
    assert_eq!(
        code(Organization::create(org_id(), "a".into(), -1)),
        "org.organization_invalid"
    );
    assert_eq!(
        code(Organization::create(org_id(), "a".into(), 253402300800000)),
        "org.organization_invalid"
    );
}

#[test]
fn owner_membership_is_explicit_and_immutable() {
    let membership = Membership::owner(membership_id(), org_id(), principal_id(), 1000).unwrap();
    let want = MembershipSnapshot {
        id: membership_id(),
        organization_id: org_id(),
        principal_id: principal_id(),
        role: Role::Owner,
        created_at_ms: 1000,
    };
    assert_eq!(membership.snapshot(), want);
    assert!(
        Membership::create(membership_id(), org_id(), principal_id(), Role::Admin, 1000).is_ok()
    );
    assert_eq!(
        code(Membership::create(
            membership_id(),
            org_id(),
            principal_id(),
            Role::Member,
            -1
        )),
        "org.membership_invalid"
    );
    let mut snapshot = membership.snapshot();
    snapshot.role = Role::Admin;
    assert_eq!(membership.snapshot(), want);
}
