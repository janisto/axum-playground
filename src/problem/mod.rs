use std::collections::BTreeSet;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::{ToResponse, ToSchema};

use crate::http::negotiation::{Representation, negotiate_problem_representation};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemCode {
    InvalidRequest,
    Unauthorized,
    Forbidden,
    ClientGeneratedIdUnsupported,
    RelationshipsUnsupported,
    NotFound,
    ProfileNotFound,
    GithubNotFound,
    MethodNotAllowed,
    NotAcceptable,
    ProfileExists,
    ProfileResourceMismatch,
    PayloadTooLarge,
    UnsupportedMediaType,
    ValidationFailed,
    RateLimited,
    GithubRateLimit,
    InternalError,
    GithubUpstream,
    DependencyUnavailable,
    GithubTimeout,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProblemSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProblemIssue {
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ProblemSource>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProblemDetails {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    pub title: String,
    #[schema(minimum = 100, maximum = 599)]
    pub status: u16,
    pub detail: String,
    pub code: ProblemCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ProblemIssue>>,
}

#[derive(Debug, ToResponse)]
#[response(
    description = "Portable Problem Details error",
    headers(
        ("X-Request-ID" = String, description = "Selected request correlation identifier"),
        ("Cache-Control" = String, description = "Always no-store"),
        ("X-Content-Type-Options" = String, description = "Always nosniff"),
        ("X-Frame-Options" = String, description = "Always DENY"),
        ("Referrer-Policy" = String, description = "Referrer policy"),
        ("Vary" = String, description = "Includes Accept")
    )
)]
pub enum ProblemResponse {
    Json(#[content("application/problem+json")] ProblemDetails),
    Cbor(#[content("application/cbor")] ProblemDetails),
}

impl ProblemCode {
    #[must_use]
    pub const fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden
            | Self::ClientGeneratedIdUnsupported
            | Self::RelationshipsUnsupported => StatusCode::FORBIDDEN,
            Self::NotFound | Self::ProfileNotFound | Self::GithubNotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::NotAcceptable => StatusCode::NOT_ACCEPTABLE,
            Self::ProfileExists | Self::ProfileResourceMismatch => StatusCode::CONFLICT,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::ValidationFailed => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited | Self::GithubRateLimit => StatusCode::TOO_MANY_REQUESTS,
            Self::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
            Self::GithubUpstream => StatusCode::BAD_GATEWAY,
            Self::DependencyUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::GithubTimeout => StatusCode::GATEWAY_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self.status().as_u16() {
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            406 => "Not Acceptable",
            409 => "Conflict",
            413 => "Content Too Large",
            415 => "Unsupported Media Type",
            422 => "Unprocessable Content",
            429 => "Too Many Requests",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            _ => "Internal Server Error",
        }
    }

    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::InvalidRequest => "Request is malformed",
            Self::Unauthorized => "Authentication is required or invalid",
            Self::Forbidden => "Access is forbidden",
            Self::ClientGeneratedIdUnsupported => "Client-generated profile IDs are not supported",
            Self::RelationshipsUnsupported => "Profile relationships are not supported",
            Self::NotFound => "Resource not found",
            Self::ProfileNotFound => "Profile not found",
            Self::GithubNotFound => "GitHub resource not found",
            Self::MethodNotAllowed => "Method not allowed",
            Self::NotAcceptable => "No acceptable response representation is available",
            Self::ProfileExists => "Profile already exists",
            Self::ProfileResourceMismatch => "Profile resource does not match this endpoint",
            Self::PayloadTooLarge => "Request body is too large",
            Self::UnsupportedMediaType => "Request representation is not supported",
            Self::ValidationFailed => "Request validation failed",
            Self::RateLimited => "Rate limit exceeded",
            Self::GithubRateLimit => "GitHub rate limit exceeded",
            Self::InternalError => "Internal server error",
            Self::GithubUpstream => "GitHub upstream response is invalid or unavailable",
            Self::DependencyUnavailable => "A required dependency is unavailable",
            Self::GithubTimeout => "GitHub request timed out",
        }
    }
}

impl ProblemDetails {
    #[must_use]
    pub fn new(code: ProblemCode) -> Self {
        Self {
            r#type: None,
            title: code.title().to_owned(),
            status: code.status().as_u16(),
            detail: code.detail().to_owned(),
            code,
            errors: None,
        }
    }

    #[must_use]
    pub fn with_errors(mut self, errors: Vec<ProblemIssue>) -> Self {
        self.errors = if errors.is_empty() {
            None
        } else if errors.len() <= 32 {
            Some(errors)
        } else {
            let mut normalized = errors.into_iter().take(31).collect::<Vec<_>>();
            normalized.push(ProblemIssue {
                detail: "Additional validation errors omitted".to_owned(),
                source: None,
            });
            Some(normalized)
        };
        self
    }

    pub fn into_response(&self, request_headers: &HeaderMap) -> Response {
        let mut problem = self.clone();
        let status =
            StatusCode::from_u16(problem.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status != problem.code.status() {
            problem = Self::new(ProblemCode::InternalError);
        }

        let representation = negotiate_problem_representation(request_headers);
        let payload = match representation {
            Representation::Cbor => {
                let mut payload = Vec::new();
                ciborium::into_writer(&problem, &mut payload)
                    .expect("serializing Problem Details to CBOR should succeed");
                payload
            }
            Representation::Json | Representation::JsonUtf8 => serde_json::to_vec(&problem)
                .expect("serializing Problem Details to JSON should succeed"),
        };
        let mut response = Response::builder()
            .status(problem.code.status())
            .header(header::CONTENT_TYPE, representation.problem_content_type())
            .body(Body::from(payload))
            .expect("Problem Details response should build");
        ensure_vary(response.headers_mut(), ["Accept"]);
        response
    }
}

#[must_use]
pub fn problem_response(code: ProblemCode, request_headers: &HeaderMap) -> Response {
    ProblemDetails::new(code).into_response(request_headers)
}

pub fn ensure_vary(headers: &mut HeaderMap, values: impl IntoIterator<Item = &'static str>) {
    let mut existing = BTreeSet::new();
    for value in headers.get_all(header::VARY) {
        if let Ok(value) = value.to_str() {
            for member in value.split(',') {
                existing.insert(member.trim().to_ascii_lowercase());
            }
        }
    }

    for value in values {
        if existing.insert(value.to_ascii_lowercase()) {
            headers.append(header::VARY, HeaderValue::from_static(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::HeaderMap};

    use super::{ProblemCode, ProblemDetails, problem_response};

    #[tokio::test]
    async fn taxonomy_response_has_exact_required_members() {
        let response = problem_response(ProblemCode::ValidationFailed, &HeaderMap::new());
        assert_eq!(response.status(), 422);
        let body = to_bytes(response.into_body(), 4096).await.expect("body");
        let problem: ProblemDetails = serde_json::from_slice(&body).expect("problem");
        assert_eq!(problem.title, "Unprocessable Content");
        assert_eq!(problem.status, 422);
        assert_eq!(problem.detail, "Request validation failed");
        assert_eq!(problem.code, ProblemCode::ValidationFailed);
    }
}
