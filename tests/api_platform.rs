mod common;

use std::{
    collections::HashSet,
    io::Write,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    routing::{get, post},
};
use axum_observability::RequestContext;
use axum_playground::{
    AuthError, MockAuthVerifier, MockGitHubService, MockProfileService, build_app,
    build_app_with_routes,
    problem::{ProblemCode, ProblemDetails},
    telemetry::observability_config,
};
use futures_util::{future::join_all, stream};
use serde_json::{Value, json};
use tower::ServiceExt;
use tracing_subscriber::prelude::*;

use crate::common::{assert_common_headers, read_json_body, state_with, test_state};

async fn panic_handler() -> &'static str {
    panic!("secret-panic-payload")
}

async fn observability_context_handler(context: RequestContext, headers: HeaderMap) -> Json<Value> {
    Json(json!({
        "requestId": context.request_id().as_str(),
        "correlationId": context.correlation_id(),
        "traceId": context.trace_context().map(|trace| trace.trace_id()),
        "requestHeader": headers.get("x-request-id").and_then(|value| value.to_str().ok()),
    }))
}

async fn raw_body_handler(request: axum::extract::Request) -> StatusCode {
    to_bytes(request.into_body(), 2_000_000)
        .await
        .map_or(StatusCode::BAD_REQUEST, |_| StatusCode::NO_CONTENT)
}

async fn failing_body_handler() -> axum::response::Response {
    axum::response::Response::new(Body::from_stream(stream::once(async {
        Err::<Bytes, _>(std::io::Error::other("secret-body-error"))
    })))
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

fn selected_id(response: &axum::response::Response) -> String {
    let values = response
        .headers()
        .get_all("x-request-id")
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 1);
    values[0].to_str().unwrap().to_owned()
}

fn assert_generated(value: &str) {
    assert_eq!(value.len(), 32);
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    );
}

#[tokio::test]
async fn request_id_accepts_the_exact_grammar_and_replaces_every_invalid_class() {
    let max = format!("A{}", "z09._:-".repeat(19));
    let max = &max[..128];
    for value in ["A", "aZ09._:-", max] {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-request-id", value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(selected_id(&response), value);
    }

    let unicode = HeaderValue::from_bytes("é".as_bytes()).expect("obs-text header should build");
    let invalid: Vec<HeaderValue> = vec![
        HeaderValue::from_static("bad id"),
        HeaderValue::from_static("bad/id"),
        HeaderValue::from_static("bad,id"),
        HeaderValue::from_static("-bad"),
        HeaderValue::from_str(&"x".repeat(129)).unwrap(),
        unicode,
    ];
    for value in invalid {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-request-id", value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_generated(&selected_id(&response));
    }

    let duplicate = build_app(test_state())
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("x-request-id", "first")
                .header("x-request-id", "second")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_generated(&selected_id(&duplicate));
}

#[tokio::test]
async fn generated_request_ids_are_concurrently_unique_and_visible_in_context() {
    let extra = Router::new().route("/__context", get(observability_context_handler));
    let app = build_app_with_routes(test_state(), extra);
    let responses = join_all((0..64).map(|_| {
        app.clone().oneshot(
            Request::builder()
                .uri("/__context")
                .body(Body::empty())
                .unwrap(),
        )
    }))
    .await;
    let mut ids = HashSet::new();
    for response in responses {
        let response = response.unwrap();
        let id = selected_id(&response);
        assert_generated(&id);
        let body: Value = read_json_body(response).await;
        assert_eq!(body["requestId"], id);
        assert_eq!(body["requestHeader"], id);
        assert_eq!(body["correlationId"], id);
        assert!(ids.insert(id));
    }
    assert_eq!(ids.len(), 64);
}

#[tokio::test]
async fn request_and_trace_context_preserve_one_selected_value() {
    let extra = Router::new().route("/__context", get(observability_context_handler));
    let trace_id = "0af7651916cd43dd8448eb211c80319c";
    let response = build_app_with_routes(test_state(), extra)
        .oneshot(
            Request::builder()
                .uri("/__context")
                .header("x-request-id", "external-id")
                .header("traceparent", format!("00-{trace_id}-b7ad6b7169203331-01"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(selected_id(&response), "external-id");
    let body: Value = read_json_body(response).await;
    assert_eq!(body["requestId"], "external-id");
    assert_eq!(body["requestHeader"], "external-id");
    assert_eq!(body["correlationId"], trace_id);
    assert_eq!(body["traceId"], trace_id);
}

#[tokio::test]
async fn representative_success_and_every_controlled_failure_preserve_request_id_and_headers() {
    let id = "representative-id";
    let mut responses = Vec::new();
    responses.push(
        build_app(test_state())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-request-id", id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    );
    responses.push(
        build_app(test_state())
            .oneshot(
                Request::builder()
                    .uri("/missing")
                    .header("x-request-id", id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    );
    responses.push(
        build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/health")
                    .header("x-request-id", id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    );
    responses.push(
        build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header("x-request-id", id)
                    .header(header::CONTENT_LENGTH, 1_000_001)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    );
    responses.push(
        build_app(test_state())
            .oneshot(
                Request::builder()
                    .uri("/v1/profile")
                    .header("x-request-id", id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    );
    responses.push(
        build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header("x-request-id", id)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":" bad"}"#))
                    .unwrap(),
            )
            .await
            .unwrap(),
    );
    let dependency_state = state_with(
        MockAuthVerifier::test_user().with_error(AuthError::ServiceUnavailable),
        MockGitHubService::demo(),
        MockProfileService::default(),
    );
    responses.push(
        build_app(dependency_state)
            .oneshot(
                Request::builder()
                    .uri("/v1/profile")
                    .header("x-request-id", id)
                    .header(header::AUTHORIZATION, "Bearer token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    );
    let panic_app = build_app_with_routes(
        test_state(),
        Router::new().route("/__panic", get(panic_handler)),
    );
    responses.push(
        panic_app
            .oneshot(
                Request::builder()
                    .uri("/__panic")
                    .header("x-request-id", id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    );

    assert_eq!(
        responses
            .iter()
            .map(axum::response::Response::status)
            .collect::<Vec<_>>(),
        [
            StatusCode::OK,
            StatusCode::NOT_FOUND,
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::PAYLOAD_TOO_LARGE,
            StatusCode::UNAUTHORIZED,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::INTERNAL_SERVER_ERROR,
        ]
    );
    for response in responses {
        assert_eq!(selected_id(&response), id);
        assert_common_headers(&response, true);
    }
}

#[tokio::test]
async fn portable_operations_do_not_expose_trailing_slash_aliases() {
    let app = build_app(test_state());
    for path in [
        "/health",
        "/v1/hello",
        "/v1/items",
        "/v1/profile",
        "/v1/github/owners/octocat",
        "/v1/github/owners/octocat/repos",
        "/v1/github/repos/octocat/hello-world",
        "/v1/github/repos/octocat/hello-world/activity",
        "/v1/github/repos/octocat/hello-world/languages",
        "/v1/github/repos/octocat/hello-world/tags",
        "/openapi.json",
    ] {
        let target = format!("{path}/");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&target)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{target}");
        assert_common_headers(&response, true);
        let problem: ProblemDetails = read_json_body(response).await;
        assert_eq!(problem.code, ProblemCode::NotFound, "{target}");
    }
}

#[tokio::test]
async fn malformed_github_paths_are_not_classified_as_registered_head_routes() {
    let app = build_app(test_state());
    for path in [
        "//v1/github/owners/octocat",
        "///v1/github/repos/octocat/hello-world",
    ] {
        for method in [Method::GET, Method::HEAD] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(path)
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("request should complete");

            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {path}");
            assert!(!response.headers().contains_key(header::ALLOW));
            assert_common_headers(&response, true);
        }
    }
}

#[tokio::test]
async fn rejected_head_requests_preserve_headers_without_sending_a_body() {
    let app = build_app(test_state());
    for (path, allow) in [
        ("/health", "GET"),
        ("/v1/hello", "GET, POST"),
        ("/v1/items", "GET"),
        ("/v1/profile", "GET, POST, PATCH, DELETE"),
        ("/v1/github/owners/octocat", "GET"),
        ("/v1/github/owners/octocat/repos", "GET"),
        ("/v1/github/repos/octocat/hello-world", "GET"),
        ("/v1/github/repos/octocat/hello-world/activity", "GET"),
        ("/v1/github/repos/octocat/hello-world/languages", "GET"),
        ("/v1/github/repos/octocat/hello-world/tags", "GET"),
        ("/openapi.json", "GET"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::HEAD)
                    .uri(path)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
        assert_eq!(response.headers()[header::ALLOW], allow, "{path}");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json",
            "{path}"
        );
        assert_common_headers(&response, true);
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body should be readable")
                .is_empty(),
            "{path}"
        );
    }

    for path in [
        "/missing",
        "/health/",
        "//v1/github/owners/octocat",
        "///v1/github/repos/octocat/hello-world",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::HEAD)
                    .uri(path)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert!(!response.headers().contains_key(header::ALLOW), "{path}");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json",
            "{path}"
        );
        assert_common_headers(&response, true);
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body should be readable")
                .is_empty(),
            "{path}"
        );
    }

    let response = build_app_with_routes(
        test_state(),
        Router::new().route("/__panic", get(panic_handler)),
    )
    .oneshot(
        Request::builder()
            .method(Method::HEAD)
            .uri("/__panic")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await
    .expect("request should complete");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_common_headers(&response, true);
    assert!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable")
            .is_empty()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn panic_recovery_returns_a_safe_problem_and_does_not_log_the_payload() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer_logs = Arc::clone(&logs);
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || LogWriter(Arc::clone(&writer_logs)))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let response = build_app_with_routes(
        test_state(),
        Router::new().route("/__panic", get(panic_handler)),
    )
    .oneshot(
        Request::builder()
            .uri("/__panic")
            .header("x-request-id", "panic-id")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(selected_id(&response), "panic-id");
    let problem: ProblemDetails = read_json_body(response).await;
    assert_eq!(problem.code, ProblemCode::InternalError);
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("request panicked"));
    assert!(!logs.contains("secret-panic-payload"));
}

#[tokio::test]
async fn request_size_policy_is_route_and_method_specific_not_global() {
    let app = build_app_with_routes(
        test_state(),
        Router::new().route("/__raw-body", post(raw_body_handler)),
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/__raw-body")
                .header(header::CONTENT_LENGTH, 1_000_001)
                .body(Body::from(vec![b'x'; 1_000_001]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let missing = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/missing")
                .header(header::CONTENT_LENGTH, 1_000_001)
                .body(Body::from(vec![b'x'; 1_000_001]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn security_headers_are_exact_and_cors_is_not_wildcard_enabled() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(header::ORIGIN, "https://example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(
        response.headers()[header::REFERRER_POLICY],
        "strict-origin-when-cross-origin"
    );
    assert_eq!(
        response.headers()["content-security-policy"],
        "default-src 'none'; frame-ancestors 'none'"
    );
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
    assert!(!response.headers().contains_key(header::SERVER));

    let preflight = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/health")
                .header(header::ORIGIN, "https://example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preflight.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(preflight.headers()[header::ALLOW], "GET");
    assert!(
        !preflight
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn observability_emits_stable_terminal_records_without_concrete_paths_or_secrets() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer_logs = Arc::clone(&logs);
    let subscriber = tracing_subscriber::registry()
        .with(observability_config().json_layer(move || LogWriter(Arc::clone(&writer_logs))));
    let _guard = tracing::subscriber::set_default(subscriber);
    let extra = Router::new()
        .route(
            "/__observability/{item}",
            get(|| async { StatusCode::NO_CONTENT }),
        )
        .route("/__observability-error", get(failing_body_handler));
    let app = build_app_with_routes(test_state(), extra);
    let trace_id = "0af7651916cd43dd8448eb211c80319c";

    let completed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/__observability/private-item?secret=value")
                .header("x-request-id", "completed-id")
                .header("traceparent", format!("00-{trace_id}-b7ad6b7169203331-03"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    to_bytes(completed.into_body(), 1024).await.unwrap();

    let failed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/__observability-error")
                .header("x-request-id", "failed-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    to_bytes(failed.into_body(), 1024)
        .await
        .expect_err("body should fail");

    let abandoned = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("x-request-id", "abandoned-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    drop(abandoned);

    let serialized = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    for forbidden in ["private-item", "secret=value", "secret-body-error"] {
        assert!(!serialized.contains(forbidden));
    }
    let records = serialized
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["target"], "axum_observability::access");
    assert_eq!(records[0]["request_id"], "completed-id");
    assert_eq!(records[0]["correlation_id"], trace_id);
    assert_eq!(records[0]["path_template"], "/__observability/{item}");
    assert!(records[0].get("path").is_none());
    assert!(records[0]["httpRequest"].get("requestUrl").is_none());
    assert_eq!(records[1]["terminal_reason"], "body_error");
    assert_eq!(records[1]["severity"], "ERROR");
    assert_eq!(records[2]["terminal_reason"], "response_dropped");
    assert_eq!(records[2]["severity"], "ERROR");
}
