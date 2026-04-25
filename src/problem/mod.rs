pub mod negotiate;

use std::collections::BTreeSet;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};

use crate::problem::negotiate::select_format;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ProblemFieldError {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ProblemFieldError>>,
}

impl ProblemDetails {
    pub fn new(status: u16, title: impl Into<String>) -> Self {
        Self {
            schema: None,
            r#type: None,
            title: Some(title.into()),
            status,
            detail: None,
            instance: None,
            errors: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn into_response(&self, request_headers: &HeaderMap, scheme: &str, host: &str) -> Response {
        let mut response_problem = self.clone();
        response_problem.schema = Some(format!("{scheme}://{host}/schemas/ErrorModel.json"));

        let prefers_cbor = select_format(
            request_headers
                .get(header::ACCEPT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default(),
        );

        let status = StatusCode::from_u16(response_problem.status)
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        response_problem.status = status.as_u16();

        let mut response = if prefers_cbor {
            let mut body = Vec::new();
            ciborium::into_writer(&response_problem, &mut body)
                .expect("serializing problem details to CBOR should succeed");
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "application/problem+cbor")
                .body(Body::from(body))
                .expect("response should build")
        } else {
            let body = serde_json::to_vec(&response_problem)
                .expect("serializing problem details to JSON should succeed");
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "application/problem+json")
                .body(Body::from(body))
                .expect("response should build")
        };

        ensure_vary(response.headers_mut(), ["Origin", "Accept"]);
        response.headers_mut().append(
            header::LINK,
            HeaderValue::from_static("</schemas/ErrorModel.json>; rel=\"describedBy\""),
        );
        response
    }
}

pub fn problem_response(
    status: StatusCode,
    detail: impl Into<String>,
    request_headers: &HeaderMap,
) -> Response {
    ProblemDetails::new(
        status.as_u16(),
        status.canonical_reason().unwrap_or("Internal Server Error"),
    )
    .with_detail(detail)
    .into_response(
        request_headers,
        &request_scheme(request_headers),
        &request_host(request_headers),
    )
}

pub fn request_scheme(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("http")
        .to_string()
}

pub fn request_host(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("localhost")
        .to_string()
}

pub fn ensure_vary(headers: &mut HeaderMap, values: impl IntoIterator<Item = &'static str>) {
    let mut existing = BTreeSet::new();

    for value in headers.get_all(header::VARY) {
        if let Ok(value) = value.to_str() {
            for part in value.split(',') {
                existing.insert(part.trim().to_ascii_lowercase());
            }
        }
    }

    for value in values {
        let normalized = value.to_ascii_lowercase();
        if existing.insert(normalized) {
            headers.append(
                header::VARY,
                HeaderValue::from_str(value).expect("vary values must be valid headers"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::IntoResponse,
    };

    use super::{ProblemDetails, ensure_vary, problem_response, request_host, request_scheme};

    #[test]
    fn ensure_vary_merges_without_duplicates() {
        let mut headers = HeaderMap::new();
        headers.append(header::VARY, HeaderValue::from_static("Accept-Encoding"));

        ensure_vary(&mut headers, ["Origin", "Accept", "Origin"]);

        let values = headers
            .get_all(header::VARY)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>();

        assert_eq!(values, vec!["Accept-Encoding", "Origin", "Accept"]);
    }

    #[test]
    fn request_metadata_prefers_forwarded_values() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("api.example.com"),
        );
        headers.insert(
            header::HOST,
            HeaderValue::from_static("ignored.example.com"),
        );

        assert_eq!(request_scheme(&headers), "https");
        assert_eq!(request_host(&headers), "api.example.com");
    }

    #[test]
    fn problem_response_uses_request_metadata_defaults() {
        let response = problem_response(
            StatusCode::NOT_FOUND,
            "resource not found",
            &HeaderMap::new(),
        )
        .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get(header::LINK)
                .and_then(|value| value.to_str().ok()),
            Some("</schemas/ErrorModel.json>; rel=\"describedBy\"")
        );
    }

    #[tokio::test]
    async fn invalid_problem_status_normalizes_body_and_http_status() {
        let response = ProblemDetails::new(42, "invalid")
            .into_response(&HeaderMap::new(), "http", "localhost")
            .into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("problem body should be readable");
        let problem: ProblemDetails =
            serde_json::from_slice(&body).expect("problem body should deserialize");

        assert_eq!(problem.status, StatusCode::INTERNAL_SERVER_ERROR.as_u16());
    }
}
