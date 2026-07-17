mod common;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
};
use axum_playground::{build_app, problem::ProblemDetails};
use tower::ServiceExt;

use common::{read_cbor_body, read_json_body, test_state};

#[tokio::test]
async fn problem_details_default_to_json_and_include_relative_schema_link() {
    let headers = HeaderMap::new();

    let response = ProblemDetails::new(404, "Not Found")
        .with_detail("missing resource")
        .into_response(&headers);

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/problem+json"))
    );
    assert_eq!(
        response.headers().get(header::LINK),
        Some(&HeaderValue::from_static(
            "</schemas/ErrorModel.json>; rel=\"describedBy\""
        ))
    );

    let vary_values = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>();
    assert_eq!(vary_values, vec!["Origin", "Accept"]);

    let problem: ProblemDetails = read_json_body(response).await;
    assert_eq!(problem.title.as_deref(), Some("Not Found"));
    assert_eq!(problem.detail.as_deref(), Some("missing resource"));
}

#[tokio::test]
async fn problem_details_use_cbor_when_accept_prefers_cbor() {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/cbor"));

    let response = ProblemDetails::new(400, "Bad Request").into_response(&headers);

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/cbor"))
    );

    let problem: ProblemDetails = read_cbor_body(response).await;
    assert_eq!(problem.title.as_deref(), Some("Bad Request"));
}

#[tokio::test]
async fn advertised_error_schema_link_resolves_to_json_schema() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .uri("/schemas/ErrorModel.json")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/schema+json")
    );
    let schema: serde_json::Value = read_json_body(response).await;
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["required"], serde_json::json!(["status"]));
}
