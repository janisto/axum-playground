mod common;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum_playground::problem::{
    ProblemCode, ProblemDetails, ProblemIssue, ProblemSource, problem_response,
};
use serde_json::Value;

use crate::common::{read_cbor_body, read_json_body};

#[tokio::test]
async fn every_problem_code_has_the_exact_status_title_detail_and_wire_name() {
    let taxonomy = [
        (
            ProblemCode::InvalidRequest,
            400,
            "Bad Request",
            "Request is malformed",
            "invalid_request",
        ),
        (
            ProblemCode::Unauthorized,
            401,
            "Unauthorized",
            "Authentication is required or invalid",
            "unauthorized",
        ),
        (
            ProblemCode::Forbidden,
            403,
            "Forbidden",
            "Access is forbidden",
            "forbidden",
        ),
        (
            ProblemCode::ClientGeneratedIdUnsupported,
            403,
            "Forbidden",
            "Client-generated profile IDs are not supported",
            "client_generated_id_unsupported",
        ),
        (
            ProblemCode::RelationshipsUnsupported,
            403,
            "Forbidden",
            "Profile relationships are not supported",
            "relationships_unsupported",
        ),
        (
            ProblemCode::NotFound,
            404,
            "Not Found",
            "Resource not found",
            "not_found",
        ),
        (
            ProblemCode::ProfileNotFound,
            404,
            "Not Found",
            "Profile not found",
            "profile_not_found",
        ),
        (
            ProblemCode::GithubNotFound,
            404,
            "Not Found",
            "GitHub resource not found",
            "github_not_found",
        ),
        (
            ProblemCode::MethodNotAllowed,
            405,
            "Method Not Allowed",
            "Method not allowed",
            "method_not_allowed",
        ),
        (
            ProblemCode::NotAcceptable,
            406,
            "Not Acceptable",
            "No acceptable response representation is available",
            "not_acceptable",
        ),
        (
            ProblemCode::ProfileExists,
            409,
            "Conflict",
            "Profile already exists",
            "profile_exists",
        ),
        (
            ProblemCode::ProfileResourceMismatch,
            409,
            "Conflict",
            "Profile resource does not match this endpoint",
            "profile_resource_mismatch",
        ),
        (
            ProblemCode::PayloadTooLarge,
            413,
            "Content Too Large",
            "Request body is too large",
            "payload_too_large",
        ),
        (
            ProblemCode::UnsupportedMediaType,
            415,
            "Unsupported Media Type",
            "Request representation is not supported",
            "unsupported_media_type",
        ),
        (
            ProblemCode::ValidationFailed,
            422,
            "Unprocessable Content",
            "Request validation failed",
            "validation_failed",
        ),
        (
            ProblemCode::RateLimited,
            429,
            "Too Many Requests",
            "Rate limit exceeded",
            "rate_limited",
        ),
        (
            ProblemCode::GithubRateLimit,
            429,
            "Too Many Requests",
            "GitHub rate limit exceeded",
            "github_rate_limit",
        ),
        (
            ProblemCode::InternalError,
            500,
            "Internal Server Error",
            "Internal server error",
            "internal_error",
        ),
        (
            ProblemCode::GithubUpstream,
            502,
            "Bad Gateway",
            "GitHub upstream response is invalid or unavailable",
            "github_upstream",
        ),
        (
            ProblemCode::DependencyUnavailable,
            503,
            "Service Unavailable",
            "A required dependency is unavailable",
            "dependency_unavailable",
        ),
        (
            ProblemCode::GithubTimeout,
            504,
            "Gateway Timeout",
            "GitHub request timed out",
            "github_timeout",
        ),
    ];
    for (code, status, title, detail, wire_code) in taxonomy {
        let response = problem_response(code, &HeaderMap::new());
        assert_eq!(response.status(), StatusCode::from_u16(status).unwrap());
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        let value: Value = read_json_body(response).await;
        assert_eq!(
            value,
            serde_json::json!({
                "title": title, "status": status, "detail": detail, "code": wire_code
            })
        );
    }
}

#[tokio::test]
async fn problem_negotiation_prefers_cbor_only_when_acceptable_and_never_changes_status() {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/cbor"));
    let response = problem_response(ProblemCode::PayloadTooLarge, &headers);
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/cbor");
    assert_eq!(
        read_cbor_body::<ProblemDetails>(response).await.code,
        ProblemCode::PayloadTooLarge
    );

    headers.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
    let response = problem_response(ProblemCode::PayloadTooLarge, &headers);
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
}

#[test]
fn problem_issue_truncation_is_bounded_and_uses_the_exact_omission_marker() {
    let issues = (0..40)
        .map(|index| ProblemIssue {
            detail: format!("safe issue {index}"),
            source: Some(ProblemSource {
                pointer: Some("/known".to_owned()),
                parameter: None,
                header: None,
            }),
        })
        .collect::<Vec<_>>();
    let problem = ProblemDetails::new(ProblemCode::ValidationFailed).with_errors(issues);
    let errors = problem.errors.expect("issues should be retained");
    assert_eq!(errors.len(), 32);
    assert_eq!(errors[30].detail, "safe issue 30");
    assert_eq!(errors[31].detail, "Additional validation errors omitted");
    assert!(errors[31].source.is_none());
}
