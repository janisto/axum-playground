mod common;

use std::convert::Infallible;

use axum::{
    body::{Body, Bytes},
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{MockAuthVerifier, MockGitHubService, MockProfileService, build_app};
use futures_util::stream;
use serde::Deserialize;
use tower::ServiceExt;

use crate::common::{
    assert_common_headers, read_cbor_body, read_json_body, state_with, test_state,
};

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct HealthResponse {
    status: String,
}

async fn request(
    method: Method,
    target: &str,
    accept: Option<&str>,
    body: Body,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(target);
    if let Some(accept) = accept {
        request = request.header(header::ACCEPT, accept);
    }
    build_app(test_state())
        .oneshot(request.body(body).expect("request should build"))
        .await
        .expect("request should complete")
}

#[tokio::test]
async fn health_returns_the_exact_json_and_cbor_liveness_document() {
    let json = request(Method::GET, "/health", None, Body::empty()).await;
    assert_eq!(json.status(), StatusCode::OK);
    assert_eq!(json.headers()[header::CONTENT_TYPE], "application/json");
    assert_common_headers(&json, true);
    for forbidden in [header::LOCATION, header::LINK, header::WWW_AUTHENTICATE] {
        assert!(!json.headers().contains_key(forbidden));
    }
    assert_eq!(
        read_json_body::<HealthResponse>(json).await,
        HealthResponse {
            status: "healthy".to_owned()
        }
    );

    let cbor = request(
        Method::GET,
        "/health",
        Some("application/cbor"),
        Body::empty(),
    )
    .await;
    assert_eq!(cbor.status(), StatusCode::OK);
    assert_eq!(cbor.headers()[header::CONTENT_TYPE], "application/cbor");
    assert_eq!(
        read_cbor_body::<HealthResponse>(cbor).await,
        HealthResponse {
            status: "healthy".to_owned()
        }
    );
}

#[tokio::test]
async fn health_is_dependency_free_and_does_not_poll_request_content() {
    let auth = MockAuthVerifier::test_user();
    let github = MockGitHubService::demo();
    let profile = MockProfileService::default();
    let state = state_with(auth.clone(), github.clone(), profile.clone());
    let body = Body::from_stream(stream::once(async {
        panic!("body-free operation polled request content");
        #[allow(unreachable_code)]
        Ok::<Bytes, Infallible>(Bytes::new())
    }));

    let response = build_app(state)
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(body)
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(auth.call_count(), 0);
    assert_eq!(github.call_count(), 0);
    assert_eq!(profile.committed_write_count(), 0);
}

#[tokio::test]
async fn health_rejects_closed_or_malformed_query_and_unacceptable_success_media() {
    for target in [
        "/health?unknown=1",
        "/health?x=1&x=2",
        "/health?x=%",
        "/health?=x",
    ] {
        let response = request(Method::GET, target, None, Body::empty()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{target}");
    }

    let response = request(Method::GET, "/health", Some("text/html"), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
}

#[tokio::test]
async fn health_unsupported_methods_are_405_with_the_exact_allow_value() {
    for method in [Method::HEAD, Method::POST, Method::OPTIONS] {
        let response = request(method.clone(), "/health", None, Body::empty()).await;
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
        assert_eq!(response.headers()[header::ALLOW], "GET");
    }
    let trailing = request(Method::GET, "/health/", None, Body::empty()).await;
    assert_eq!(trailing.status(), StatusCode::NOT_FOUND);
}
