use std::collections::BTreeSet;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::{ToResponse, ToSchema};

use crate::http::negotiation::{
    CBOR_MEDIA_TYPE, PROBLEM_JSON_MEDIA_TYPE, Representation, negotiate_problem_representation,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct ProblemFieldError {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct ProblemDetails {
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

#[derive(ToResponse)]
#[response(description = "Problem Details error")]
pub enum ProblemResponse {
    Json(#[content("application/problem+json")] ProblemDetails),
    Cbor(#[content("application/cbor")] ProblemDetails),
}

impl ProblemDetails {
    pub fn new(status: u16, title: impl Into<String>) -> Self {
        Self {
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

    pub fn into_response(&self, request_headers: &HeaderMap) -> Response {
        let mut response_problem = self.clone();
        let status = StatusCode::from_u16(response_problem.status)
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        response_problem.status = status.as_u16();

        let mut response = match negotiate_problem_representation(request_headers) {
            Representation::Cbor => {
                let mut body = Vec::new();
                ciborium::into_writer(&response_problem, &mut body)
                    .expect("serializing problem details to CBOR should succeed");
                Response::builder()
                    .status(status)
                    .header(header::CONTENT_TYPE, CBOR_MEDIA_TYPE)
                    .body(Body::from(body))
                    .expect("response should build")
            }
            Representation::Json => {
                let body = serde_json::to_vec(&response_problem)
                    .expect("serializing problem details to JSON should succeed");
                Response::builder()
                    .status(status)
                    .header(header::CONTENT_TYPE, PROBLEM_JSON_MEDIA_TYPE)
                    .body(Body::from(body))
                    .expect("response should build")
            }
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
    .into_response(request_headers)
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
    };

    use super::{ProblemDetails, ensure_vary, problem_response};

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
    fn problem_response_includes_relative_schema_link() {
        let response = problem_response(
            StatusCode::NOT_FOUND,
            "resource not found",
            &HeaderMap::new(),
        );

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
        let response = ProblemDetails::new(42, "invalid").into_response(&HeaderMap::new());

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("problem body should be readable");
        let problem: ProblemDetails =
            serde_json::from_slice(&body).expect("problem body should deserialize");

        assert_eq!(problem.status, StatusCode::INTERNAL_SERVER_ERROR.as_u16());
    }
}
