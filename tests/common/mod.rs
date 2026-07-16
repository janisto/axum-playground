#![allow(dead_code)]

use axum::{
    body::{Body, to_bytes},
    http::Response,
};
use std::sync::Arc;

use axum_playground::{AppConfig, AppState, AuthVerifier, GitHubService, ProfileService};
use serde::de::DeserializeOwned;

pub async fn read_json_body<T: DeserializeOwned>(response: Response<Body>) -> T {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    serde_json::from_slice(&body).expect("body should deserialize from JSON")
}

pub async fn read_cbor_body<T: DeserializeOwned>(response: Response<Body>) -> T {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    ciborium::from_reader(body.as_ref()).expect("body should deserialize from CBOR")
}

pub async fn read_text_body(response: Response<Body>) -> String {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    String::from_utf8(body.to_vec()).expect("body should be valid UTF-8")
}

pub fn test_state() -> Arc<AppState> {
    test_state_with_github_service(GitHubService::mock(
        axum_playground::MockGitHubService::demo(),
    ))
}

pub fn test_state_with_github_service(github_service: GitHubService) -> Arc<AppState> {
    Arc::new(AppState::with_services(
        base_test_config(),
        github_service,
        AuthVerifier::mock(axum_playground::MockAuthVerifier::test_user()),
        ProfileService::mock(axum_playground::MockProfileService::default()),
    ))
}

pub fn test_state_with_auth_and_profile(
    auth_verifier: AuthVerifier,
    profile_service: ProfileService,
) -> Arc<AppState> {
    Arc::new(AppState::with_services(
        base_test_config(),
        GitHubService::mock(axum_playground::MockGitHubService::demo()),
        auth_verifier,
        profile_service,
    ))
}

fn base_test_config() -> AppConfig {
    AppConfig {
        port: 8080,
        firebase_project_id: "demo-test-project".to_string(),
        app_environment: "test".to_string(),
        github_token: None,
        google_application_credentials: None,
        firebase_auth_emulator_host: None,
        firestore_emulator_host: None,
        google_cloud_project: Some("demo-test-project".to_string()),
        gcp_project: None,
        gcloud_project: None,
        project_id: None,
    }
}
