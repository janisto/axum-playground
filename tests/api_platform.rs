mod common;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    routing::get,
};
use axum_playground::{build_app, build_app_with_routes, problem::ProblemDetails};
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, test_state};

async fn panic_handler() -> &'static str {
    panic!("boom")
}

#[tokio::test]
async fn not_found_returns_problem_details_and_request_id() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/missing")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    assert!(response.headers().contains_key("x-request-id"));
    assert!(response.headers().contains_key(header::LINK));

    let body: ProblemDetails = read_json_body(response).await;
    assert_eq!(body.status, StatusCode::NOT_FOUND.as_u16());
    assert_eq!(body.title.as_deref(), Some("Not Found"));
    assert_eq!(body.detail.as_deref(), Some("resource not found"));
}

#[tokio::test]
async fn not_found_honors_cbor_negotiation() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/missing")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+cbor")
    );

    let body: ProblemDetails = read_cbor_body(response).await;
    assert_eq!(body.title.as_deref(), Some("Not Found"));
}

#[tokio::test]
async fn method_not_allowed_returns_problem_details_and_allow_header() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/health")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        response
            .headers()
            .get(header::ALLOW)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("GET"))
    );

    let body: ProblemDetails = read_json_body(response).await;
    assert_eq!(body.title.as_deref(), Some("Method Not Allowed"));
    assert_eq!(body.detail.as_deref(), Some("method POST not allowed"));
}

#[tokio::test]
async fn panic_recovery_returns_internal_server_error_problem() {
    let extra_routes = Router::new().route("/__panic", get(panic_handler));

    let response = build_app_with_routes(test_state(), extra_routes)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/__panic")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body: ProblemDetails = read_json_body(response).await;
    assert_eq!(body.title.as_deref(), Some("Internal Server Error"));
    assert_eq!(body.detail.as_deref(), Some("internal server error"));
}

#[tokio::test]
async fn cors_preflight_uses_api_defaults() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/health")
                .header(header::ORIGIN, "https://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
    assert!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("GET"))
    );
}
