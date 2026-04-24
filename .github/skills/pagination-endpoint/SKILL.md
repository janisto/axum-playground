---
name: pagination-endpoint
description: Guide for creating cursor-paginated endpoints in this Axum project using the shared pagination helpers and RFC 8288 Link headers.
---

# Pagination Endpoint Creation

Use this skill when creating paginated list endpoints for this Axum REST API application.

## Pagination Package

The project uses cursor-based pagination via `src/pagination`:
- `pagination::cursor::decode_cursor(...)`
- `pagination::cursor::Cursor`
- `pagination::paginate(...)`
- `pagination::link::build_link_header(...)`

`paginate(...)` returns:
- `items`
- `total`
- `link_header`
- `next_cursor`
- `prev_cursor`

## Query Pattern

Define a dedicated query struct with `cursor`, `limit`, and endpoint-specific filters.

```rust
#[derive(Debug, Deserialize)]
pub struct WidgetsListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub category: Option<String>,
}
```

Use constants for cursor type and limit policy:

```rust
const WIDGET_CURSOR_KIND: &str = "widget";
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: i64 = 100;
```

## Handler Pattern

```rust
use axum::{
    extract::Query,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};

use crate::{
    http::codec::success_response_with_headers,
    pagination::{cursor::decode_cursor, paginate},
    problem::problem_response,
};

pub async fn list_widgets_handler(
    headers: HeaderMap,
    Query(query): Query<WidgetsListQuery>,
) -> Response {
    if let Some(limit) = query.limit
        && limit > MAX_LIMIT
    {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
    }

    let cursor = match decode_cursor(query.cursor.as_deref().unwrap_or_default()) {
        Ok(cursor) => cursor,
        Err(_) => {
            return problem_response(StatusCode::BAD_REQUEST, "invalid cursor format", &headers);
        }
    };

    if !cursor.kind.is_empty() && cursor.kind != WIDGET_CURSOR_KIND {
        return problem_response(StatusCode::BAD_REQUEST, "cursor type mismatch", &headers);
    }

    let widgets = all_widgets();

    if !cursor.value.is_empty() && !widgets.iter().any(|widget| widget.id == cursor.value) {
        return problem_response(
            StatusCode::BAD_REQUEST,
            "cursor references unknown item",
            &headers,
        );
    }

    let query_pairs = query
        .category
        .iter()
        .map(|category| ("category".to_string(), category.clone()))
        .collect::<Vec<_>>();

    let page = paginate(
        &widgets,
        &cursor,
        query.limit.unwrap_or(DEFAULT_LIMIT as i64) as usize,
        WIDGET_CURSOR_KIND,
        |widget| widget.id.as_str(),
        "/v1/widgets",
        &query_pairs,
    );

    let extra_headers = (!page.link_header.is_empty())
        .then(|| HeaderValue::from_str(&page.link_header).expect("link header should be valid"))
        .map(|value| vec![(header::LINK, value)])
        .unwrap_or_default();

    success_response_with_headers(
        StatusCode::OK,
        &headers,
        &page.items,
        extra_headers,
    )
}
```

## Cursor Validation Rules

Invalid cursors must return `400 Bad Request`:
- malformed or undecodable cursor
- cursor type mismatch
- cursor references an unknown item

Invalid `limit` values must return `422 Unprocessable Entity` when they violate endpoint constraints.

## Link Header Contract

The `Link` header must follow RFC 8288 and keep filters in the generated URL. Always assert it in tests when the endpoint is paginated.

## Testing Pattern

```rust
#[tokio::test]
async fn list_widgets_sets_next_link() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/widgets?limit=5")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);

    let link = response
        .headers()
        .get(header::LINK)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    assert!(link.contains("rel=\"next\""));
}
```

## Checklist

1. Define a dedicated cursor kind constant.
2. Validate `limit` and filters before pagination.
3. Decode and validate the cursor before slicing data.
4. Preserve filter query parameters in pagination links.
5. Add focused tests for success, invalid cursor, and limit validation.