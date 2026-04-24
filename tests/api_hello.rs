mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{build_app, problem::ProblemDetails};
use serde::Deserialize;
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, read_text_body, test_state};

#[derive(Debug, Deserialize)]
struct HelloData {
    message: String,
}

#[tokio::test]
async fn get_hello_supports_json_and_cbor() {
    let json_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/hello")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(json_response.status(), StatusCode::OK);
    assert_eq!(
        json_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let json_body: HelloData = read_json_body(json_response).await;
    assert_eq!(json_body.message, "Hello, World!");

    let cbor_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/hello")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(cbor_response.status(), StatusCode::OK);
    assert_eq!(
        cbor_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/cbor")
    );
    let cbor_body: HelloData = read_cbor_body(cbor_response).await;
    assert_eq!(cbor_body.message, "Hello, World!");
}

#[tokio::test]
async fn post_hello_supports_json_and_cbor_requests() {
    let json_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Test"}"#))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(json_response.status(), StatusCode::CREATED);
    let json_body: HelloData = read_json_body(json_response).await;
    assert_eq!(json_body.message, "Hello, Test!");

    let mut cbor_payload = Vec::new();
    ciborium::into_writer(&serde_json::json!({"name": "CBOR"}), &mut cbor_payload)
        .expect("CBOR payload should serialize");

    let cbor_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/cbor")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::from(cbor_payload))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(cbor_response.status(), StatusCode::CREATED);
    assert_eq!(
        cbor_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/cbor")
    );
    let cbor_body: HelloData = read_cbor_body(cbor_response).await;
    assert_eq!(cbor_body.message, "Hello, CBOR!");
}

#[tokio::test]
async fn post_hello_validation_errors_follow_accept_negotiation() {
    let json_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":""}"#))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(json_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    let json_problem: ProblemDetails = read_json_body(json_response).await;
    assert_eq!(
        json_problem.status,
        StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    );

    let mut cbor_payload = Vec::new();
    ciborium::into_writer(&serde_json::json!({"name": ""}), &mut cbor_payload)
        .expect("CBOR payload should serialize");

    let cbor_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/cbor")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::from(cbor_payload))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(cbor_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        cbor_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+cbor")
    );
    let cbor_problem: ProblemDetails = read_cbor_body(cbor_response).await;
    assert_eq!(
        cbor_problem.status,
        StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    );
}

#[tokio::test]
async fn openapi_includes_hello_paths_and_media_types() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/openapi")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    let body = read_text_body(response).await;
    assert!(body.contains("\"/v1/hello\""));
    assert!(body.contains("application/json"));
    assert!(body.contains("application/cbor"));
}
