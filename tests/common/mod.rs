#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    body::{Body, Bytes, to_bytes},
    http::Response,
};
use axum_playground::{
    AppConfig, AppEnvironment, AppState, AuthVerifier, GitHubService, MockAuthVerifier,
    MockGitHubService, MockProfileService, ProfileService,
};
use serde::de::DeserializeOwned;

const TEST_RESPONSE_BODY_LIMIT: usize = 8 * 1024 * 1024;

pub(crate) async fn body_bytes(response: Response<Body>) -> Bytes {
    to_bytes(response.into_body(), TEST_RESPONSE_BODY_LIMIT)
        .await
        .expect("response body should be readable")
}

pub(crate) async fn read_json_body<T: DeserializeOwned>(response: Response<Body>) -> T {
    serde_json::from_slice(&body_bytes(response).await).expect("response should be valid JSON")
}

pub(crate) async fn read_cbor_body<T: DeserializeOwned>(response: Response<Body>) -> T {
    ciborium::from_reader(body_bytes(response).await.as_ref())
        .expect("response should be valid CBOR")
}

pub(crate) async fn read_text_body(response: Response<Body>) -> String {
    String::from_utf8(body_bytes(response).await.to_vec())
        .expect("response body should be valid UTF-8")
}

pub(crate) fn test_state() -> Arc<AppState> {
    state_with(
        MockAuthVerifier::test_user(),
        MockGitHubService::demo(),
        MockProfileService::default(),
    )
}

pub(crate) fn state_with(
    auth: MockAuthVerifier,
    github: MockGitHubService,
    profile: MockProfileService,
) -> Arc<AppState> {
    Arc::new(AppState::with_services(
        base_test_config(),
        GitHubService::mock(github),
        AuthVerifier::mock(auth),
        ProfileService::mock(profile),
    ))
}

pub(crate) fn base_test_config() -> AppConfig {
    AppConfig {
        port: 8080,
        firebase_project_id: "demo-test-project".to_owned(),
        app_environment: AppEnvironment::Test,
        google_application_credentials: None,
        firebase_auth_emulator_host: None,
        firestore_emulator_host: None,
        google_cloud_project: Some("demo-test-project".to_owned()),
        gcp_project: None,
        gcloud_project: None,
        project_id: None,
    }
}

pub(crate) fn assert_common_headers(response: &Response<Body>, vary: bool) {
    let headers = response.headers();
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        headers
            .get("x-frame-options")
            .and_then(|value| value.to_str().ok()),
        Some("DENY")
    );
    assert_eq!(
        headers
            .get("referrer-policy")
            .and_then(|value| value.to_str().ok()),
        Some("strict-origin-when-cross-origin")
    );
    assert!(headers.get("x-request-id").is_some());
    if vary {
        assert!(
            headers
                .get_all("vary")
                .iter()
                .filter_map(|value| value.to_str().ok())
                .any(|value| value.eq_ignore_ascii_case("Accept"))
        );
    }
}

pub(crate) fn bearer_request(
    builder: axum::http::request::Builder,
) -> axum::http::request::Builder {
    builder.header("authorization", "Bearer test-token")
}
