mod common;

use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::{get, post},
};
use axum_observability::RequestContext;
use axum_playground::{build_app, build_app_with_routes, problem::ProblemDetails};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, test_state};

async fn panic_handler() -> &'static str {
    panic!("secret-panic-payload")
}

#[derive(Debug)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("log buffer should lock")
            .extend(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn observability_context_handler(context: RequestContext, headers: HeaderMap) -> Json<Value> {
    Json(json!({
        "requestId": context.request_id().as_str(),
        "correlationId": context.correlation_id(),
        "traceId": context.trace_context().map(|trace| trace.trace_id()),
        "requestHeader": headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
    }))
}

async fn raw_body_handler(request: axum::extract::Request) -> StatusCode {
    match to_bytes(request.into_body(), usize::MAX).await {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::PAYLOAD_TOO_LARGE,
    }
}

async fn assert_payload_too_large_problem(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    let problem: ProblemDetails = read_json_body(response).await;
    assert_eq!(problem.status, StatusCode::PAYLOAD_TOO_LARGE.as_u16());
    assert_eq!(problem.detail.as_deref(), Some("request body is too large"));
}

#[tokio::test]
async fn observability_context_preserves_valid_request_and_trace_ids() {
    let extra_routes = Router::new().route("/__observability", get(observability_context_handler));
    let trace_id = "0af7651916cd43dd8448eb211c80319c";

    let response = build_app_with_routes(test_state(), extra_routes)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/__observability")
                .header("x-request-id", "external-id")
                .header("traceparent", format!("00-{trace_id}-b7ad6b7169203331-01"))
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

    let body: Value = read_json_body(response).await;
    assert_eq!(body["requestId"], "external-id");
    assert_eq!(body["requestHeader"], "external-id");
    assert_eq!(body["correlationId"], trace_id);
    assert_eq!(body["traceId"], trace_id);
}

#[tokio::test]
async fn observability_context_replaces_duplicate_request_ids_once() {
    let extra_routes = Router::new().route("/__observability", get(observability_context_handler));

    let response = build_app_with_routes(test_state(), extra_routes)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/__observability")
                .header("x-request-id", "first-id")
                .header("x-request-id", "second-id")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("response request ID should be present")
        .to_owned();
    assert_eq!(request_id.len(), 32);
    assert!(
        request_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );

    let body: Value = read_json_body(response).await;
    assert_eq!(body["requestId"], request_id);
    assert_eq!(body["requestHeader"], request_id);
    assert_eq!(body["correlationId"], request_id);
    assert!(body["traceId"].is_null());
}

#[tokio::test]
async fn global_body_limit_covers_raw_body_consumers() {
    const BODY_LIMIT: usize = 1024 * 1024;

    let extra_routes = Router::new().route("/__raw-body", post(raw_body_handler));
    let app = build_app_with_routes(test_state(), extra_routes);

    let at_limit = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/__raw-body")
                .body(Body::from(vec![b'x'; BODY_LIMIT]))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(at_limit.status(), StatusCode::NO_CONTENT);

    let oversized = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/__raw-body")
                .body(Body::from(vec![b'x'; BODY_LIMIT + 1]))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_payload_too_large_problem(oversized).await;

    let declared_oversized = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/__raw-body")
                .header(header::CONTENT_LENGTH, BODY_LIMIT + 1)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_payload_too_large_problem(declared_oversized).await;
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
        Some("application/cbor")
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

#[tokio::test(flavor = "current_thread")]
async fn panic_recovery_returns_internal_server_error_problem() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer_logs = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || LogWriter(writer_logs.clone()))
        .finish();
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);
    let extra_routes = Router::new().route("/__panic", get(panic_handler));

    let response = build_app_with_routes(test_state(), extra_routes)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/__panic")
                .header("x-request-id", "panic-id")
                .header(header::ORIGIN, "https://example.com")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("panic-id")
    );
    assert_eq!(
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );

    let body: ProblemDetails = read_json_body(response).await;
    assert_eq!(body.title.as_deref(), Some("Internal Server Error"));
    assert_eq!(body.detail.as_deref(), Some("internal server error"));

    let logs = String::from_utf8(logs.lock().expect("log buffer should lock").clone())
        .expect("logs should be UTF-8");
    assert!(logs.contains("request panicked"));
    assert!(!logs.contains("secret-panic-payload"));
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
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "traceparent,tracestate,x-request-id",
                )
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
    let allowed_headers = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|value| value.to_str().ok())
        .expect("CORS response should list allowed headers");
    assert!(allowed_headers.contains("traceparent"));
    assert!(allowed_headers.contains("tracestate"));
    assert!(allowed_headers.contains("x-request-id"));
}

#[tokio::test]
async fn cors_exposes_authentication_and_method_contract_headers() {
    let unauthorized = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/profile")
                .header(header::ORIGIN, "https://example.com")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unauthorized
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer")
    );
    let exposed_headers = unauthorized
        .headers()
        .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
        .and_then(|value| value.to_str().ok())
        .expect("CORS response should list exposed headers");
    assert!(exposed_headers.contains("www-authenticate"));

    let method_not_allowed = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/health")
                .header(header::ORIGIN, "https://example.com")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(method_not_allowed.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(method_not_allowed.headers().contains_key(header::ALLOW));
    let exposed_headers = method_not_allowed
        .headers()
        .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
        .and_then(|value| value.to_str().ok())
        .expect("CORS response should list exposed headers");
    assert!(exposed_headers.contains("allow"));
}

#[tokio::test]
async fn swagger_ui_receives_compatible_security_headers() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api-docs/")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        response
            .headers()
            .get(header::X_FRAME_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("DENY")
    );
}
