use n2f_rs::{
    domains::org::{
        app::{CreateOrganizationRecord, CreateOutcome, OrganizationStore},
        domain::{Membership, Organization},
        infra::postgres::{Store, migration},
    },
    shared::{
        errors::Classified,
        events::postgres::migration as events_migration,
        id::Id,
        postgres::{Config, Database},
        secret::SecretString,
    },
};
use std::{sync::Arc, time::Duration};

fn id(value: &str) -> Id {
    Id::parse(&format!("00000000-0000-4000-8000-000000000{value}")).unwrap()
}

#[tokio::test]
#[ignore = "requires N2F_TEST_DATABASE_URL pointing at disposable PostgreSQL 18"]
async fn create_organization_is_atomic_against_postgres() {
    let url = std::env::var("N2F_TEST_DATABASE_URL").expect("database URL");
    let database = Arc::new(
        Database::open(Config {
            url: SecretString::new(url),
            max_connections: 4,
            timeout: Duration::from_secs(5),
        })
        .await
        .unwrap(),
    );
    database
        .migrate(vec![events_migration(1), migration(6)])
        .await
        .unwrap();
    let store = Store::new(database.clone());
    let organization = Organization::create(id("061"), "Persistence Team".into(), 1000).unwrap();
    let owner = Membership::owner(id("062"), id("061"), id("063"), 1000).unwrap();
    let record = CreateOrganizationRecord {
        organization: organization.snapshot(),
        owner: owner.snapshot(),
    };
    assert_eq!(
        store.create_organization(record.clone()).await.unwrap(),
        CreateOutcome::Created
    );
    assert_eq!(
        store.create_organization(record.clone()).await.unwrap(),
        CreateOutcome::AlreadyExists
    );
    let counts = database
        .transaction(|tx| {
            Box::pin(async move {
                let organizations: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM public.n2f_org_organizations")
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(n2f_rs::shared::postgres::map)?;
                let memberships: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM public.n2f_org_memberships")
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(n2f_rs::shared::postgres::map)?;
                Ok((organizations, memberships))
            })
        })
        .await
        .unwrap();
    assert_eq!(counts, (1, 1));
    let mismatched_owner = Membership::owner(id("064"), id("065"), id("063"), 1000).unwrap();
    let invalid = CreateOrganizationRecord {
        organization: organization.snapshot(),
        owner: mismatched_owner.snapshot(),
    };
    assert_eq!(
        store
            .create_organization(invalid)
            .await
            .unwrap_err()
            .classification()
            .unwrap()
            .error_type,
        Some("org.store_invalid_record")
    );
}
