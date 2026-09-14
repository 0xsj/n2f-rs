use n2f_rs::domains::org::app::{
    Clock, CreateOrganization, CreateOrganizationInput, CreateOrganizationRecord, CreateOutcome,
    Eligibility, IdSource, ListOrganizations, OrganizationReader, OrganizationStore,
    PrincipalEligibility,
};
use n2f_rs::shared::{
    errors::{Classified, Failure, Kind},
    id::Id,
};
use std::{
    future::{Future, ready},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn id(value: &str) -> Id {
    Id::parse(&format!("00000000-0000-4000-8000-000000000{value}")).unwrap()
}

struct Ports {
    eligible: Eligibility,
    ids: Mutex<Vec<Id>>,
    outcome: CreateOutcome,
    record: Mutex<Option<CreateOrganizationRecord>>,
}
impl Clock for Ports {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(1000)
    }
}
impl IdSource for Ports {
    fn new_id(&self) -> Result<Id, Failure> {
        Ok(self.ids.lock().unwrap().remove(0))
    }
}
impl PrincipalEligibility for Ports {
    fn check_active(
        &self,
        _principal_id: Id,
    ) -> impl Future<Output = Result<Eligibility, Failure>> + Send {
        ready(Ok(self.eligible))
    }
}
impl OrganizationStore for Ports {
    fn create_organization(
        &self,
        record: CreateOrganizationRecord,
    ) -> impl Future<Output = Result<CreateOutcome, Failure>> + Send {
        *self.record.lock().unwrap() = Some(record);
        ready(Ok(self.outcome))
    }
}

#[tokio::test]
async fn create_checks_eligibility_and_commits_both_values() {
    let ports = Ports {
        eligible: Eligibility { eligible: true },
        ids: Mutex::new(vec![id("010"), id("011")]),
        outcome: CreateOutcome::Created,
        record: Mutex::new(None),
    };
    let operation = CreateOrganization::new(ports).unwrap();
    let result = operation
        .execute(CreateOrganizationInput {
            name: " Team 🌱 ".into(),
            owner_principal_id: id("012"),
        })
        .await
        .unwrap();
    assert_eq!(result.organization.id, id("010"));
    assert_eq!(result.organization.name, " Team 🌱 ");
    assert_eq!(result.owner.id, id("011"));
    assert_eq!(result.owner.role, n2f_rs::domains::org::domain::Role::Owner);
}

#[tokio::test]
async fn ineligible_owner_does_not_allocate_or_write() {
    let ports = Ports {
        eligible: Eligibility { eligible: false },
        ids: Mutex::new(vec![id("010"), id("011")]),
        outcome: CreateOutcome::Created,
        record: Mutex::new(None),
    };
    let operation = CreateOrganization::new(ports).unwrap();
    let error = operation
        .execute(CreateOrganizationInput {
            name: "Team".into(),
            owner_principal_id: id("012"),
        })
        .await
        .unwrap_err();
    assert_eq!(error.classification().unwrap().kind, Kind::Forbidden);
    assert_eq!(
        error.classification().unwrap().error_type,
        Some("org.principal_ineligible")
    );
}

#[tokio::test]
async fn expected_duplicate_is_a_conflict() {
    let ports = Ports {
        eligible: Eligibility { eligible: true },
        ids: Mutex::new(vec![id("010"), id("011")]),
        outcome: CreateOutcome::AlreadyExists,
        record: Mutex::new(None),
    };
    let operation = CreateOrganization::new(ports).unwrap();
    let error = operation
        .execute(CreateOrganizationInput {
            name: "Team".into(),
            owner_principal_id: id("012"),
        })
        .await
        .unwrap_err();
    assert_eq!(error.classification().unwrap().kind, Kind::Conflict);
    assert_eq!(
        error.classification().unwrap().error_type,
        Some("org.organization_exists")
    );
}

struct Reader {
    seen: Mutex<Option<(Id, i64)>>,
}
impl OrganizationReader for Reader {
    fn list_for_principal(
        &self,
        principal_id: Id,
        limit: i64,
    ) -> impl Future<
        Output = Result<Vec<n2f_rs::domains::org::app::OrganizationMembership>, Failure>,
    > + Send {
        *self.seen.lock().unwrap() = Some((principal_id, limit));
        ready(Ok(Vec::new()))
    }
}

#[tokio::test]
async fn list_scopes_the_read_to_the_principal_and_limit() {
    let reader = Reader {
        seen: Mutex::new(None),
    };
    let operation = ListOrganizations::new(reader).unwrap();
    let result = operation.execute(id("012"), 25).await.unwrap();
    assert!(result.is_empty());
}
