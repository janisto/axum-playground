use std::time::Duration;

use axum_playground::{
    AppConfig, AppState, CreateProfileParams, ProfileServiceError, UpdateProfileParams,
    profile_migration::{ProfileMigrationErrorKind, ProfileMigrationMode, run_profile_migration},
};
use serde_json::{Value, json};

const EMULATOR_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const EMULATOR_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const PROJECT_ID: &str = "demo-test-project";
const USER_ID: &str = "tenant/user";

#[tokio::test]
#[ignore = "requires FIRESTORE_EMULATOR_HOST"]
async fn firestore_profile_service_and_migration_round_trip_when_emulator_is_configured() {
    let config = emulator_config();
    let emulator_host = config
        .firestore_emulator_host
        .clone()
        .expect("FIRESTORE_EMULATOR_HOST must be set for the emulator test");
    assert_emulator_reachable(&emulator_host).await;

    flush_emulator(&emulator_host, PROJECT_ID).await;
    verify_migration_preflight_is_write_free(&config, &emulator_host).await;
    flush_emulator(&emulator_host, PROJECT_ID).await;
    verify_migration_round_trip(&config, &emulator_host).await;
    flush_emulator(&emulator_host, PROJECT_ID).await;

    let state = AppState::new(config).expect("loopback emulator configuration should be valid");

    let service = &state.profile_service;

    let created = service
        .create(
            USER_ID,
            CreateProfileParams {
                first_name: "John".to_owned(),
                last_name: "Doe".to_owned(),
                contact_email: "John@example.com".to_owned(),
                phone_number: "+358401234567".to_owned(),
                marketing_opt_in: true,
                terms_accepted: true,
            },
        )
        .await
        .expect("create should succeed against emulator");

    assert_eq!(created.contact_email, "John@example.com");
    assert_eq!(created.phone_number, "+358401234567");
    assert_eq!(created.id, USER_ID);

    let duplicate = service
        .create(
            USER_ID,
            CreateProfileParams {
                first_name: "Jane".to_owned(),
                last_name: "Doe".to_owned(),
                contact_email: "jane@example.com".to_owned(),
                phone_number: "+358401234567".to_owned(),
                marketing_opt_in: false,
                terms_accepted: true,
            },
        )
        .await
        .expect_err("duplicate create should fail against emulator");
    assert!(matches!(duplicate, ProfileServiceError::AlreadyExists));

    let fetched = service
        .get(USER_ID)
        .await
        .expect("get should succeed against emulator");
    assert_eq!(fetched.first_name, "John");

    let updated = service
        .update(
            USER_ID,
            UpdateProfileParams {
                first_name: Some("Jane".to_owned()),
                contact_email: Some("UPDATED@example.com".to_owned()),
                marketing_opt_in: Some(false),
                ..UpdateProfileParams::default()
            },
        )
        .await
        .expect("update should succeed against emulator");

    assert_eq!(updated.first_name, "Jane");
    assert_eq!(updated.contact_email, "UPDATED@example.com");
    assert!(!updated.marketing_opt_in);

    service
        .delete(USER_ID)
        .await
        .expect("delete should succeed against emulator");

    let missing = service
        .get(USER_ID)
        .await
        .expect_err("deleted profile should not be found");
    assert!(matches!(missing, ProfileServiceError::NotFound));

    flush_emulator(&emulator_host, PROJECT_ID).await;
}

async fn verify_migration_preflight_is_write_free(config: &AppConfig, host: &str) {
    create_legacy_document(host, "uid_dGVuYW50L3VzZXI", USER_ID, false).await;
    create_legacy_document(host, "uid_YmxvY2tlZA", "blocked", true).await;

    let error = run_profile_migration(
        config,
        ProfileMigrationMode::Apply {
            confirmed_project: PROJECT_ID.to_owned(),
        },
    )
    .await
    .expect_err("mixed persisted shape must block the full preflight");
    assert_eq!(error.kind(), ProfileMigrationErrorKind::BlockedPreflight);
    let report = error.report().expect("blocked audit report");
    assert_eq!(report.migrated, 0);
    assert_eq!(report.migration_required(), 1);
    assert_eq!(report.blocked(), 1);

    let unchanged = get_document(host, "uid_dGVuYW50L3VzZXI").await;
    assert!(unchanged["fields"].get("firstname").is_some());
    assert!(unchanged["fields"].get("firstName").is_none());
}

async fn verify_migration_round_trip(config: &AppConfig, host: &str) {
    create_legacy_document(host, "uid_dGVuYW50L3VzZXI", USER_ID, false).await;
    let audit = run_profile_migration(config, ProfileMigrationMode::Audit)
        .await
        .expect("migration audit");
    assert_eq!(audit.migration_required(), 1);
    assert_eq!(audit.current(), 0);

    let applied = run_profile_migration(
        config,
        ProfileMigrationMode::Apply {
            confirmed_project: PROJECT_ID.to_owned(),
        },
    )
    .await
    .expect("migration apply");
    assert_eq!(applied.migrated, 1);
    assert_eq!(applied.current(), 1);
    assert_eq!(applied.migration_required(), 0);

    let canonical = get_document(host, "uid_dGVuYW50L3VzZXI").await;
    assert!(canonical["fields"].get("firstName").is_some());
    assert!(canonical["fields"].get("firstname").is_none());

    let idempotent = run_profile_migration(
        config,
        ProfileMigrationMode::Apply {
            confirmed_project: PROJECT_ID.to_owned(),
        },
    )
    .await
    .expect("idempotent migration apply");
    assert_eq!(idempotent.migrated, 0);
    assert_eq!(idempotent.current(), 1);
}

async fn create_legacy_document(host: &str, document_id: &str, id: &str, mixed: bool) {
    let mut fields = json!({
        "id": {"stringValue": id},
        "firstname": {"stringValue": "Ada"},
        "lastname": {"stringValue": "Lovelace"},
        "email": {"stringValue": "ada@example.com"},
        "phoneNumber": {"stringValue": "+358401234567"},
        "marketing": {"booleanValue": false},
        "terms": {"booleanValue": true},
        "createdAt": {"stringValue": "2026-07-30T12:00:00.123456Z"},
        "updatedAt": {"stringValue": "2026-07-30T12:05:00.999999Z"}
    });
    if mixed {
        fields["firstName"] = json!({"stringValue": "Ada"});
    }
    let url = format!(
        "http://{host}/v1/projects/{PROJECT_ID}/databases/(default)/documents/profiles?documentId={document_id}"
    );
    let response = reqwest::Client::new()
        .post(url)
        .json(&json!({"fields": fields}))
        .send()
        .await
        .expect("seed legacy profile");
    assert!(
        response.status().is_success(),
        "legacy seed: {}",
        response.status()
    );
}

async fn get_document(host: &str, document_id: &str) -> Value {
    let url = format!(
        "http://{host}/v1/projects/{PROJECT_ID}/databases/(default)/documents/profiles/{document_id}"
    );
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .expect("read profile document");
    assert!(
        response.status().is_success(),
        "document read: {}",
        response.status()
    );
    response.json().await.expect("Firestore document JSON")
}

fn emulator_config() -> AppConfig {
    AppConfig {
        port: 8080,
        firebase_project_id: PROJECT_ID.to_owned(),
        app_environment: axum_playground::AppEnvironment::Test,
        google_application_credentials: None,
        firebase_auth_emulator_host: Some("127.0.0.1:9099".to_owned()),
        firestore_emulator_host: emulator_host(),
        google_cloud_project: Some(PROJECT_ID.to_owned()),
        gcp_project: None,
        gcloud_project: None,
        project_id: None,
    }
}

fn emulator_host() -> Option<String> {
    std::env::var("FIRESTORE_EMULATOR_HOST")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn assert_emulator_reachable(host: &str) {
    tokio::time::timeout(
        EMULATOR_CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect(host),
    )
    .await
    .expect("connecting to Firestore emulator timed out")
    .expect("connecting to Firestore emulator should succeed");
}

async fn flush_emulator(host: &str, project_id: &str) {
    let url =
        format!("http://{host}/emulator/v1/projects/{project_id}/databases/(default)/documents");

    let client = reqwest::Client::builder()
        .timeout(EMULATOR_REQUEST_TIMEOUT)
        .build()
        .expect("emulator flush client should build");

    let response = client
        .delete(url)
        .send()
        .await
        .expect("emulator flush request should succeed");

    assert!(
        response.status().is_success(),
        "emulator flush should return success, got {}",
        response.status()
    );
}
