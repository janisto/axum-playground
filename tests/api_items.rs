mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{build_app, pagination::cursor::Cursor, problem::ProblemDetails};
use serde::Deserialize;
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, read_text_body, test_state};

#[derive(Debug, Deserialize)]
struct Item {
    id: String,
    category: String,
}

#[derive(Debug, Deserialize)]
struct ItemsResponse {
    items: Vec<Item>,
    total: usize,
}

#[tokio::test]
async fn list_items_returns_first_page_with_next_link() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let link_header = response
        .headers()
        .get(header::LINK)
        .and_then(|value| value.to_str().ok())
        .expect("link header should exist")
        .to_owned();
    let body: ItemsResponse = read_json_body(response).await;

    assert_eq!(body.items.len(), 20);
    assert_eq!(body.total, 30);
    assert_eq!(
        body.items.first().map(|item| item.id.as_str()),
        Some("item-001")
    );
    assert!(link_header.contains("rel=\"next\""));
    assert!(!link_header.contains("rel=\"prev\""));
}

#[tokio::test]
async fn list_items_middle_page_has_prev_and_next_links() {
    let cursor = Cursor::new("item", "item-010").encode();
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/items?cursor={cursor}&limit=5"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let link_header = response
        .headers()
        .get(header::LINK)
        .and_then(|value| value.to_str().ok())
        .expect("link header should exist")
        .to_owned();
    let body: ItemsResponse = read_json_body(response).await;

    assert_eq!(body.items.len(), 5);
    assert_eq!(
        body.items.first().map(|item| item.id.as_str()),
        Some("item-011")
    );
    assert!(link_header.contains("rel=\"next\""));
    assert!(link_header.contains("rel=\"prev\""));
}

#[tokio::test]
async fn list_items_preserves_category_and_supports_cbor() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?category=tools&limit=3")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let link_header = response
        .headers()
        .get(header::LINK)
        .and_then(|value| value.to_str().ok())
        .expect("link header should exist")
        .to_owned();
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/cbor")
    );
    let body: ItemsResponse = read_cbor_body(response).await;

    assert_eq!(body.items.len(), 3);
    assert!(body.items.iter().all(|item| item.category == "tools"));
    assert!(link_header.contains("category=tools"));
}

#[tokio::test]
async fn list_items_rejects_invalid_cursor_and_category() {
    let invalid_cursor = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?cursor=invalid!!!")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(invalid_cursor.status(), StatusCode::BAD_REQUEST);
    let invalid_cursor_body: ProblemDetails = read_json_body(invalid_cursor).await;
    assert_eq!(invalid_cursor_body.status, StatusCode::BAD_REQUEST.as_u16());

    let invalid_category = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?category=nonexistent")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(invalid_category.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_category_body: ProblemDetails = read_json_body(invalid_category).await;
    assert_eq!(
        invalid_category_body.status,
        StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    );
}

#[tokio::test]
async fn list_items_rejects_non_positive_limit() {
    let zero_limit = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?limit=0")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(zero_limit.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let zero_limit_body: ProblemDetails = read_json_body(zero_limit).await;
    assert_eq!(
        zero_limit_body.status,
        StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    );

    let negative_limit = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?limit=-10")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(negative_limit.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let negative_limit_body: ProblemDetails = read_json_body(negative_limit).await;
    assert_eq!(
        negative_limit_body.status,
        StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    );
}

#[tokio::test]
async fn list_items_enforces_limit_and_cursor_boundaries() {
    let maximum = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?limit=100")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(maximum.status(), StatusCode::OK);
    let body: ItemsResponse = read_json_body(maximum).await;
    assert_eq!(body.items.len(), 30);

    let above_maximum = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/items?limit=101")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(above_maximum.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let untyped_cursor = Cursor::new("", "item-001").encode();
    let untyped = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/items?cursor={untyped_cursor}"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(untyped.status(), StatusCode::BAD_REQUEST);

    let stale_cursor = Cursor::new("item", "item-001").encode();
    let stale = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/items?category=tools&cursor={stale_cursor}"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn openapi_includes_items_path() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/openapi")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    let body = read_text_body(response).await;
    assert!(body.contains("\"/v1/items\""));
    assert!(body.contains("application/cbor"));
}
