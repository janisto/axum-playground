---
name: rust-testing
description: Guide for writing Rust tests for this Axum project using in-process router tests, shared test state, and focused validation patterns.
---

# Rust Testing

Use this skill when writing or updating tests for this Axum REST API application.

## Test Organization

The project uses integration tests under `tests/` plus local unit tests inside modules when the logic is tightly scoped:

```text
tests/
    api_health.rs
    api_hello.rs
    api_items.rs
    api_github.rs
    api_platform.rs
    api_profile.rs
    firestore_emulator.rs
    problem_details.rs
    common/
        mod.rs
src/
    pagination/
        mod.rs   # local unit tests are fine here
```

## Test App Setup

Use the shared helpers in `tests/common/mod.rs` and exercise the app in-process through `build_app(...)`.

```rust
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use axum_playground::build_app;
use tower::ServiceExt;

use crate::common::{read_json_body, test_state};

#[derive(serde::Deserialize)]
struct HealthResponse {
    status: String,
}

#[tokio::test]
async fn health_endpoint_returns_expected_payload() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);

    let body: HealthResponse = read_json_body(response).await;
    assert_eq!(body.status, "healthy");
}
```

## Shared Helpers

Prefer the helpers in `tests/common/mod.rs`:
- `test_state()` for the default app state
- `test_state_with_github_service(...)` for GitHub-specific tests
- `test_state_with_auth_and_profile(...)` for auth/profile slices
- `read_json_body(...)`, `read_cbor_body(...)`, and `read_text_body(...)`

## Basic Endpoint Pattern

```rust
#[tokio::test]
async fn get_hello_returns_default_message() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/hello")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
}
```

## Testing POST Requests

```rust
#[tokio::test]
async fn create_hello_returns_created() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"Rust"}"#))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::CREATED);
}
```

## Testing Error Responses

For invalid input, unknown routes, or bad cursors, verify both the status code and the Problem Details contract.

```rust
#[tokio::test]
async fn invalid_cursor_returns_bad_request() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?cursor=invalid")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

## Testing Content Negotiation

Versioned endpoints negotiate JSON and CBOR. Verify the response content type and decode with the matching helper.

```rust
#[tokio::test]
async fn hello_supports_cbor() {
    let response = build_app(test_state())
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

    assert_eq!(response.status(), StatusCode::OK);
}
```

## Testing Link Headers

Paginated endpoints should assert the `Link` header, not just the item count.

```rust
#[tokio::test]
async fn items_include_pagination_links() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?limit=5")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    let link = response
        .headers()
        .get(header::LINK)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    assert!(link.contains("rel=\"next\""));
}
```

## Focused Validation Commands

Prefer the narrowest relevant command first:
- `cargo test --locked --test api_health`
- `cargo test --locked --test api_hello`
- `cargo test --locked --test api_items`
- `cargo test --locked --test api_profile`
- `cargo test --locked --test firestore_emulator`
- `cargo nextest run --locked`
- `just test`

## Guidelines

- Prefer one behavior per test.
- Use route-level integration tests for HTTP behavior and unit tests for pure helpers.
- Assert headers when they are part of the contract.
- Reuse shared helpers instead of duplicating response decoding logic.
- Keep emulator-backed tests isolated and conditional.