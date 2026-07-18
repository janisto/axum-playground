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
    let vary = json_response
        .headers()
        .get_all(header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    assert_eq!(vary.iter().filter(|value| *value == "origin").count(), 1);
    assert_eq!(vary.iter().filter(|value| *value == "accept").count(), 1);
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
                .body(Body::from(r#"{"name":"  Test  "}"#))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(json_response.status(), StatusCode::CREATED);
    let json_body: HelloData = read_json_body(json_response).await;
    assert_eq!(json_body.message, "Hello, Test!");

    let mut cbor_payload = Vec::new();
    ciborium::into_writer(&serde_json::json!({"name": "  CBOR  "}), &mut cbor_payload)
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
        Some("application/cbor")
    );
    let cbor_problem: ProblemDetails = read_cbor_body(cbor_response).await;
    assert_eq!(
        cbor_problem.status,
        StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    );
}

#[tokio::test]
async fn post_hello_rejects_blank_and_control_character_names() {
    for name in ["   ", "Jane\nDoe", "Jane\u{7f}Doe"] {
        let body = serde_json::to_vec(&serde_json::json!({"name": name}))
            .expect("request body should serialize");
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{name:?}"
        );
    }
}

#[tokio::test]
async fn hello_rejects_unacceptable_success_representations_before_body_parsing() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::ACCEPT, "text/html")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not json"))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    let problem: ProblemDetails = read_json_body(response).await;
    assert_eq!(problem.status, StatusCode::NOT_ACCEPTABLE.as_u16());
}

#[tokio::test]
async fn post_hello_rejects_missing_or_unowned_content_types() {
    for content_type in [None, Some("application/example+cbor")] {
        let mut request = Request::builder().method(Method::POST).uri("/v1/hello");
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }

        let response = build_app(test_state())
            .oneshot(
                request
                    .body(Body::from(r#"{"name":"Test"}"#))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let problem: ProblemDetails = read_json_body(response).await;
        assert_eq!(problem.status, StatusCode::UNSUPPORTED_MEDIA_TYPE.as_u16());
    }
}

#[tokio::test]
async fn post_hello_validates_media_type_before_empty_body_syntax() {
    for (content_type, expected_status, expected_detail) in [
        (
            "text/plain",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported request media type",
        ),
        (
            "application/json",
            StatusCode::BAD_REQUEST,
            "invalid request body",
        ),
    ] {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), expected_status, "{content_type}");
        let problem: ProblemDetails = read_json_body(response).await;
        assert_eq!(problem.status, expected_status.as_u16(), "{content_type}");
        assert_eq!(
            problem.detail.as_deref(),
            Some(expected_detail),
            "{content_type}"
        );
    }
}

#[tokio::test]
async fn post_hello_rejects_trailing_cbor_items_and_oversized_bodies() {
    let mut cbor_payload = Vec::new();
    ciborium::into_writer(&serde_json::json!({"name": "CBOR"}), &mut cbor_payload)
        .expect("CBOR payload should serialize");
    cbor_payload.push(0xf6);

    let trailing_item = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/cbor")
                .body(Body::from(cbor_payload))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(trailing_item.status(), StatusCode::BAD_REQUEST);

    let oversized = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(vec![b'x'; 1024 * 1024 + 1]))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let problem: ProblemDetails = read_json_body(oversized).await;
    assert_eq!(problem.status, StatusCode::PAYLOAD_TOO_LARGE.as_u16());
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
    let document: serde_json::Value =
        serde_json::from_str(&body).expect("OpenAPI document should be JSON");
    let post = &document["paths"]["/v1/hello"]["post"];
    let name_schema = &document["components"]["schemas"]["HelloCreateBody"]["properties"]["name"];
    assert_eq!(name_schema["minLength"], 1);
    assert_eq!(name_schema["maxLength"], 100);
    assert_eq!(name_schema["pattern"], r".*\S.*");

    assert_eq!(
        post["requestBody"]["content"]
            .as_object()
            .expect("request content should be an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["application/cbor", "application/json"]
    );
    assert_eq!(
        post["responses"]["201"]["content"]
            .as_object()
            .expect("success content should be an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["application/cbor", "application/json"]
    );
    assert_eq!(
        post["responses"]["406"]["$ref"],
        "#/components/responses/ProblemResponse"
    );
    assert_eq!(
        document["components"]["responses"]["ProblemResponse"]["content"]
            .as_object()
            .expect("problem content should be an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["application/cbor", "application/problem+json"]
    );
}
