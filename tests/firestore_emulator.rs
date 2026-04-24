use axum_playground::{
    AppConfig, CreateProfileParams, ProfileService, ProfileServiceError, UpdateProfileParams,
};

const PROJECT_ID: &str = "demo-test-project";

#[tokio::test]
async fn firestore_profile_service_crud_round_trip_when_emulator_is_configured() {
    let Some(emulator_host) = emulator_host() else {
        return;
    };

    flush_emulator(&emulator_host, PROJECT_ID).await;

    let service = ProfileService::firestore(&emulator_config());

    let created = service
        .create(
            "user-123",
            CreateProfileParams {
                firstname: "John".to_string(),
                lastname: "Doe".to_string(),
                email: "JOHN@EXAMPLE.COM".to_string(),
                phone_number: " +358401234567 ".to_string(),
                marketing: true,
                terms: true,
            },
        )
        .await
        .expect("create should succeed against emulator");

    assert_eq!(created.email, "john@example.com");
    assert_eq!(created.phone_number, "+358401234567");

    let duplicate = service
        .create(
            "user-123",
            CreateProfileParams {
                firstname: "Jane".to_string(),
                lastname: "Doe".to_string(),
                email: "jane@example.com".to_string(),
                phone_number: "+358401234567".to_string(),
                marketing: false,
                terms: true,
            },
        )
        .await
        .expect_err("duplicate create should fail against emulator");
    assert_eq!(duplicate, ProfileServiceError::AlreadyExists);

    let fetched = service
        .get("user-123")
        .await
        .expect("get should succeed against emulator");
    assert_eq!(fetched.firstname, "John");

    let updated = service
        .update(
            "user-123",
            UpdateProfileParams {
                firstname: Some("Jane".to_string()),
                email: Some("UPDATED@EXAMPLE.COM".to_string()),
                marketing: Some(false),
                ..UpdateProfileParams::default()
            },
        )
        .await
        .expect("update should succeed against emulator");

    assert_eq!(updated.firstname, "Jane");
    assert_eq!(updated.email, "updated@example.com");
    assert!(!updated.marketing);

    service
        .delete("user-123")
        .await
        .expect("delete should succeed against emulator");

    let missing = service
        .get("user-123")
        .await
        .expect_err("deleted profile should not be found");
    assert_eq!(missing, ProfileServiceError::NotFound);

    flush_emulator(&emulator_host, PROJECT_ID).await;
}

fn emulator_config() -> AppConfig {
    AppConfig {
        port: 8080,
        firebase_project_id: PROJECT_ID.to_string(),
        app_environment: "test-firestore-emulator".to_string(),
        github_token: None,
        google_application_credentials: None,
        firebase_auth_emulator_host: None,
        firestore_emulator_host: emulator_host(),
        google_cloud_project: Some(PROJECT_ID.to_string()),
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

async fn flush_emulator(host: &str, project_id: &str) {
    let host = host
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let url =
        format!("http://{host}/emulator/v1/projects/{project_id}/databases/(default)/documents");

    let response = reqwest::Client::new()
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
