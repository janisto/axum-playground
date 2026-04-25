use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

pub const MAX_REQUEST_ID_LENGTH: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestId(String);

impl RequestId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

pub fn is_valid_request_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_REQUEST_ID_LENGTH {
        return false;
    }

    id.as_bytes()
        .iter()
        .all(|byte| (0x20..=0x7E).contains(byte))
}

pub async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(x_request_id_header())
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_valid_request_id(value))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    request
        .extensions_mut()
        .insert(RequestId::new(request_id.clone()));

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        x_request_id_header(),
        HeaderValue::from_str(&request_id).expect("request ID must be valid ASCII"),
    );
    response
}

fn x_request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

#[cfg(test)]
mod tests {
    use axum::{
        Extension, Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        middleware::from_fn,
        routing::get,
    };
    use tower::ServiceExt;

    use super::{MAX_REQUEST_ID_LENGTH, RequestId, is_valid_request_id, request_id_middleware};

    async fn echo_request_id(Extension(request_id): Extension<RequestId>) -> String {
        request_id.into_inner()
    }

    fn test_app() -> Router {
        Router::new()
            .route("/", get(echo_request_id))
            .layer(from_fn(request_id_middleware))
    }

    #[test]
    fn request_id_validation_matches_printable_ascii_contract() {
        assert!(!is_valid_request_id(""));
        assert!(is_valid_request_id("traceparent-123"));
        assert!(is_valid_request_id("YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo="));
        assert!(!is_valid_request_id("contains\nnewline"));
        assert!(!is_valid_request_id("contains\u{80}high-byte"));
        assert!(!is_valid_request_id(&"a".repeat(MAX_REQUEST_ID_LENGTH + 1)));
    }

    #[tokio::test]
    async fn preserves_valid_incoming_request_id() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("x-request-id", "external-id")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("external-id")
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        assert_eq!(body, "external-id");
    }

    #[tokio::test]
    async fn replaces_invalid_request_id_with_uuid() {
        let invalid_request_id = "x".repeat(MAX_REQUEST_ID_LENGTH + 1);

        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("x-request-id", &invalid_request_id)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        let header = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .expect("response header should be set")
            .to_string();

        assert_ne!(header, invalid_request_id);
        assert!(uuid::Uuid::parse_str(&header).is_ok());

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        assert_eq!(body, header);
    }
}
