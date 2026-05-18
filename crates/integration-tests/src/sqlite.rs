use mk_database::create_repository_service;
use sea_orm::Database;

use crate::context::TestContext;

pub async fn setup() -> TestContext {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let repository_service = create_repository_service(db).await.unwrap();
    let core_services = mk_core::create_services(
        mk_core::test_support::default_external_services_builder()
            .repository_service(repository_service.clone())
            .build()
            .unwrap(),
    )
    .unwrap();

    TestContext::new(core_services, repository_service, ())
}
