mod common;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum_playground::problem::ProblemDetails;

use common::{read_cbor_body, read_json_body};

#[tokio::test]
async fn problem_details_default_to_json_and_include_schema_metadata() {
    let headers = HeaderMap::new();

    let response = ProblemDetails::new(404, "Not Found")
        .with_detail("missing resource")
        .into_response(&headers, "https", "example.com");

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
    assert_eq!(
        problem.schema.as_deref(),
        Some("https://example.com/schemas/ErrorModel.json")
    );
    assert_eq!(problem.title.as_deref(), Some("Not Found"));
    assert_eq!(problem.detail.as_deref(), Some("missing resource"));
}

#[tokio::test]
async fn problem_details_use_cbor_when_accept_prefers_cbor() {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/cbor"));

    let response =
        ProblemDetails::new(400, "Bad Request").into_response(&headers, "http", "localhost:8080");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/problem+cbor"))
    );

    let problem: ProblemDetails = read_cbor_body(response).await;
    assert_eq!(
        problem.schema.as_deref(),
        Some("http://localhost:8080/schemas/ErrorModel.json")
    );
    assert_eq!(problem.title.as_deref(), Some("Bad Request"));
}
