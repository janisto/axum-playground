---
name: axum-endpoint
description: Guide for creating Axum endpoints in this project using Router modules, shared response helpers, validation, and Utoipa documentation.
---

# Axum Endpoint Creation

Use this skill when creating new endpoints for this Axum REST API application.

## Router Setup

Create route modules in `src/http/v1/` and merge them in `src/http/v1/mod.rs`:

```rust
pub mod widgets;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(docs::router())
        .merge(hello::router())
        .merge(items::router())
        .merge(widgets::router())
}
```

If the endpoint belongs at the root rather than under `/v1`, wire it in `src/app.rs`.

## Route Module Pattern

Each route module should expose `pub fn router() -> Router<Arc<AppState>>` and register only its own paths.

```rust
use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    http::codec::success_response,
    state::AppState,
};

#[derive(Debug, Serialize, ToSchema)]
pub struct WidgetData {
    pub id: String,
    pub name: String,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/widgets", get(list_widgets_handler))
}

pub async fn list_widgets_handler(headers: HeaderMap) -> Response {
    success_response(
        StatusCode::OK,
        &headers,
        &vec![WidgetData {
            id: "widget-001".to_string(),
            name: "Widget".to_string(),
        }],
    )
}
```

## OpenAPI Documentation

Document handlers with `#[utoipa::path(...)]` and keep the path value aligned with the external URL.

```rust
#[utoipa::path(
    get,
    path = "/v1/widgets",
    tag = "Widgets",
    responses(
        (status = 200, description = "List widgets", content(
            (Vec<WidgetData> = "application/json"),
            (Vec<WidgetData> = "application/cbor")
        ))
    )
)]
```

## Request Body Pattern

Decode request bodies through `decode_request_body(...)` so JSON and CBOR behavior stays consistent.

```rust
use axum::body::Bytes;
use serde::Deserialize;
use validator::Validate;

use crate::{
    http::codec::decode_request_body,
    problem::problem_response,
};

#[derive(Debug, Deserialize, Validate, utoipa::ToSchema)]
pub struct WidgetCreateBody {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
}

pub async fn create_widget_handler(headers: HeaderMap, body: Bytes) -> Response {
    let input = match decode_request_body::<WidgetCreateBody>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };

    if input.validate().is_err() {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
    }

    success_response(
        StatusCode::CREATED,
        &headers,
        &WidgetData {
            id: "widget-001".to_string(),
            name: input.name,
        },
    )
}
```

## Header-Aware Responses

Use `success_response_with_headers(...)` when the contract includes `Link`, `Location`, or other headers.

## Error Handling

Use `problem_response(...)` for explicit status mappings.

Common status patterns in this project:
- `400 Bad Request` for malformed cursors or invalid request shapes
- `401 Unauthorized` for auth failures
- `404 Not Found` for missing resources
- `409 Conflict` for duplicate writes
- `422 Unprocessable Entity` for validation failures

## State Access

Use `State<Arc<AppState>>` only when the handler needs services or configuration. Keep handlers stateless when possible.

## Endpoint Checklist

1. Add the route module and merge it in `src/http/v1/mod.rs`.
2. Add `utoipa` documentation for every external route.
3. Use shared response helpers for JSON/CBOR consistency.
4. Reuse `problem_response(...)` for explicit error paths.
5. Add focused integration tests in `tests/`.