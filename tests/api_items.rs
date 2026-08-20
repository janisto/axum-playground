mod common;

use std::convert::Infallible;

use axum::{
    body::{Body, Bytes},
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{
    MockAuthVerifier, MockGitHubService, MockProfileService, build_app,
    pagination::cursor::{Cursor, CursorDirection, CursorScope},
    problem::{ProblemCode, ProblemDetails},
};
use futures_util::stream;
use serde::Deserialize;
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, state_with, test_state};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Money {
    amount_minor: u64,
    currency: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Item {
    id: String,
    name: String,
    category: String,
    price: Money,
    in_stock: bool,
    created_at: String,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemPage {
    items: Vec<Item>,
    total: u64,
}

async fn get(target: &str) -> axum::response::Response {
    build_app(test_state())
        .oneshot(Request::builder().uri(target).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn assert_problem(target: &str, status: StatusCode, code: ProblemCode) {
    let response = get(target).await;
    assert_eq!(response.status(), status, "{target}");
    let body: ProblemDetails = read_json_body(response).await;
    assert_eq!(body.status, status.as_u16());
    assert_eq!(body.code, code);
}

fn relation(link: &str, name: &str) -> Option<String> {
    link.split(", ").find_map(|member| {
        member
            .ends_with(&format!("; rel=\"{name}\""))
            .then(|| {
                member
                    .strip_prefix('<')?
                    .split_once('>')
                    .map(|(target, _)| target.to_owned())
            })
            .flatten()
    })
}

#[tokio::test]
async fn items_exposes_the_exact_fixed_catalog_and_money_model() {
    let response = get("/v1/items?limit=100").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key(header::LINK));
    let page: ItemPage = read_json_body(response).await;
    assert_eq!(page.total, 30);
    assert_eq!(page.items.len(), 30);

    let expected = [
        (
            "item-001",
            "Alpha Widget",
            "electronics",
            2999,
            true,
            "2024-01-15T10:30:00.000Z",
            "A versatile electronic widget for everyday use",
        ),
        (
            "item-002",
            "Beta Gadget",
            "electronics",
            4999,
            true,
            "2024-01-16T11:00:00.000Z",
            "Advanced gadget with smart features",
        ),
        (
            "item-003",
            "Gamma Tool",
            "tools",
            1550,
            false,
            "2024-01-17T09:15:00.000Z",
            "Precision tool for professional work",
        ),
        (
            "item-004",
            "Delta Component",
            "electronics",
            899,
            true,
            "2024-01-18T14:45:00.000Z",
            "Essential component for electronics projects",
        ),
        (
            "item-005",
            "Epsilon Sensor",
            "electronics",
            3499,
            true,
            "2024-01-19T08:00:00.000Z",
            "High-precision environmental sensor",
        ),
        (
            "item-006",
            "Zeta Cable",
            "accessories",
            1299,
            true,
            "2024-01-20T16:30:00.000Z",
            "Premium quality data cable",
        ),
        (
            "item-007",
            "Eta Adapter",
            "accessories",
            999,
            false,
            "2024-01-21T10:00:00.000Z",
            "Universal power adapter",
        ),
        (
            "item-008",
            "Theta Board",
            "electronics",
            8999,
            true,
            "2024-01-22T11:30:00.000Z",
            "Development board for prototyping",
        ),
        (
            "item-009",
            "Iota Switch",
            "electronics",
            599,
            true,
            "2024-01-23T09:45:00.000Z",
            "Tactile push button switch",
        ),
        (
            "item-010",
            "Kappa Display",
            "electronics",
            4599,
            true,
            "2024-01-24T13:00:00.000Z",
            "OLED display module",
        ),
        (
            "item-011",
            "Lambda Motor",
            "robotics",
            2499,
            true,
            "2024-01-25T08:30:00.000Z",
            "DC motor for robotics projects",
        ),
        (
            "item-012",
            "Mu Servo",
            "robotics",
            1899,
            false,
            "2024-01-26T15:00:00.000Z",
            "High-torque servo motor",
        ),
        (
            "item-013",
            "Nu Battery",
            "power",
            1499,
            true,
            "2024-01-27T10:15:00.000Z",
            "Rechargeable lithium battery pack",
        ),
        (
            "item-014",
            "Xi Charger",
            "power",
            2299,
            true,
            "2024-01-28T11:45:00.000Z",
            "Smart battery charger",
        ),
        (
            "item-015",
            "Omicron Relay",
            "electronics",
            799,
            true,
            "2024-01-29T09:00:00.000Z",
            "5V relay module",
        ),
        (
            "item-016",
            "Pi Controller",
            "electronics",
            5599,
            true,
            "2024-01-30T14:30:00.000Z",
            "Microcontroller board",
        ),
        (
            "item-017",
            "Rho Resistor Kit",
            "components",
            1199,
            true,
            "2024-02-01T08:00:00.000Z",
            "Assorted resistor pack",
        ),
        (
            "item-018",
            "Sigma Capacitor Set",
            "components",
            1399,
            true,
            "2024-02-02T10:30:00.000Z",
            "Electrolytic capacitor assortment",
        ),
        (
            "item-019",
            "Tau LED Pack",
            "components",
            699,
            true,
            "2024-02-03T11:00:00.000Z",
            "Multi-color LED assortment",
        ),
        (
            "item-020",
            "Upsilon Wire Set",
            "accessories",
            899,
            false,
            "2024-02-04T09:15:00.000Z",
            "Jumper wire kit",
        ),
        (
            "item-021",
            "Phi Breadboard",
            "tools",
            499,
            true,
            "2024-02-05T13:45:00.000Z",
            "Solderless breadboard",
        ),
        (
            "item-022",
            "Chi Soldering Iron",
            "tools",
            3599,
            true,
            "2024-02-06T10:00:00.000Z",
            "Temperature-controlled soldering station",
        ),
        (
            "item-023",
            "Psi Multimeter",
            "tools",
            4299,
            true,
            "2024-02-07T11:30:00.000Z",
            "Digital multimeter with auto-ranging",
        ),
        (
            "item-024",
            "Omega Oscilloscope",
            "tools",
            29999,
            true,
            "2024-02-08T14:00:00.000Z",
            "Portable digital oscilloscope",
        ),
        (
            "item-025",
            "Alpha Pro Widget",
            "electronics",
            5999,
            true,
            "2024-02-09T08:30:00.000Z",
            "Professional-grade widget with extended features",
        ),
        (
            "item-026",
            "Beta Max Gadget",
            "electronics",
            7999,
            false,
            "2024-02-10T09:00:00.000Z",
            "Maximum performance gadget",
        ),
        (
            "item-027",
            "Gamma Plus Tool",
            "tools",
            2599,
            true,
            "2024-02-11T10:15:00.000Z",
            "Enhanced precision tool",
        ),
        (
            "item-028",
            "Delta Ultra Component",
            "electronics",
            1699,
            true,
            "2024-02-12T11:45:00.000Z",
            "Ultra-reliable component",
        ),
        (
            "item-029",
            "Epsilon HD Sensor",
            "electronics",
            5499,
            true,
            "2024-02-13T13:00:00.000Z",
            "High-definition sensor array",
        ),
        (
            "item-030",
            "Zeta Premium Cable",
            "accessories",
            1999,
            true,
            "2024-02-14T15:30:00.000Z",
            "Gold-plated premium cable",
        ),
    ];
    for (item, expected) in page.items.iter().zip(expected) {
        assert_eq!(item.id, expected.0);
        assert_eq!(item.name, expected.1);
        assert_eq!(item.category, expected.2);
        assert_eq!(
            item.price,
            Money {
                amount_minor: expected.3,
                currency: "USD".to_owned()
            }
        );
        assert_eq!(item.in_stock, expected.4);
        assert_eq!(item.created_at, expected.5);
        assert_eq!(item.description, expected.6);
    }
}

#[tokio::test]
async fn items_traverses_forward_and_backward_without_skips_or_duplicates() {
    let mut target = "/v1/items?limit=10".to_owned();
    let mut forward = Vec::new();
    let last_link = loop {
        let response = get(&target).await;
        assert_eq!(response.status(), StatusCode::OK);
        let link = response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let page: ItemPage = read_json_body(response).await;
        forward.extend(page.items.iter().map(|item| item.id.clone()));
        let Some(next) = relation(&link, "next") else {
            break link;
        };
        target = next;
    };
    assert_eq!(
        forward,
        (1..=30)
            .map(|value| format!("item-{value:03}"))
            .collect::<Vec<_>>()
    );
    assert!(relation(&last_link, "next").is_none());

    let middle_target =
        relation(&last_link, "prev").expect("terminal page should navigate backward");
    let middle = get(&middle_target).await;
    let middle_link = middle.headers()[header::LINK].to_str().unwrap().to_owned();
    let middle_page: ItemPage = read_json_body(middle).await;
    assert_eq!(
        middle_page.items.first().map(|item| item.id.as_str()),
        Some("item-011")
    );
    assert_eq!(
        middle_page.items.last().map(|item| item.id.as_str()),
        Some("item-020")
    );
    assert!(relation(&middle_link, "next").is_some());

    let first = get(&relation(&middle_link, "prev").unwrap()).await;
    let first_link = first.headers()[header::LINK].to_str().unwrap().to_owned();
    let first_page: ItemPage = read_json_body(first).await;
    assert_eq!(
        first_page.items.first().map(|item| item.id.as_str()),
        Some("item-001")
    );
    assert!(relation(&first_link, "prev").is_none());
}

#[tokio::test]
async fn items_filters_before_pagination_and_preserves_scope_in_relative_links() {
    let totals = [
        ("electronics", 13_u64),
        ("tools", 6),
        ("accessories", 4),
        ("robotics", 2),
        ("power", 2),
        ("components", 3),
    ];
    for (category, total) in totals {
        let response = get(&format!("/v1/items?category={category}&limit=2")).await;
        let link = response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let page: ItemPage = read_json_body(response).await;
        assert_eq!(page.total, total, "{category}");
        assert!(page.items.iter().all(|item| item.category == category));
        if total > 2 {
            let link = link.expect("multi-page filter should have navigation");
            assert!(link.starts_with("</v1/items?"));
            assert!(link.contains(&format!("category={category}")));
            assert!(link.contains("limit=2"));
            assert!(!link.contains("http://"));
            assert!(!link.contains("https://"));
        }
    }

    let cbor = build_app(test_state())
        .oneshot(
            Request::builder()
                .uri("/v1/items?category=tools&limit=2")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cbor.headers()[header::CONTENT_TYPE], "application/cbor");
    assert_eq!(read_cbor_body::<ItemPage>(cbor).await.total, 6);
}

#[tokio::test]
async fn items_separates_query_syntax_and_cursor_failures_from_typed_validation() {
    for target in [
        "/v1/items?unknown=1",
        "/v1/items?limit=1&limit=2",
        "/v1/items?cursor=%",
        "/v1/items?cursor=%FF",
        "/v1/items?cursor=",
        "/v1/items?cursor=with%20space",
    ] {
        assert_problem(target, StatusCode::BAD_REQUEST, ProblemCode::InvalidRequest).await;
    }
    let too_long = format!("/v1/items?cursor={}", "x".repeat(2049));
    assert_problem(
        &too_long,
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;

    for target in [
        "/v1/items?limit=0",
        "/v1/items?limit=101",
        "/v1/items?limit=-1",
        "/v1/items?limit=1.0",
        "/v1/items?category=TOOLS",
        "/v1/items?category=",
    ] {
        assert_problem(
            target,
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
        )
        .await;
    }
}

#[tokio::test]
async fn items_rejects_every_changed_or_stale_cursor_scope() {
    let scope = CursorScope {
        operation: "listItems",
        owner: None,
        repository: None,
        limit: 10,
        category: None,
    };
    let cursor = Cursor::new(&scope, CursorDirection::Next, "item-010").encode();
    assert_eq!(
        get(&format!("/v1/items?limit=10&cursor={cursor}"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_problem(
        &format!("/v1/items?limit=20&cursor={cursor}"),
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;
    assert_problem(
        &format!("/v1/items?limit=10&category=tools&cursor={cursor}"),
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;

    let wrong_operation = Cursor::new(
        &CursorScope {
            operation: "getHello",
            ..scope
        },
        CursorDirection::Next,
        "item-010",
    )
    .encode();
    assert_problem(
        &format!("/v1/items?limit=10&cursor={wrong_operation}"),
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;

    let stale = Cursor::new(&scope, CursorDirection::Next, "item-999").encode();
    assert_problem(
        &format!("/v1/items?limit=10&cursor={stale}"),
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;
}

#[tokio::test]
async fn items_is_dependency_free_does_not_consume_a_body_and_has_exact_method_behavior() {
    let auth = MockAuthVerifier::test_user();
    let github = MockGitHubService::demo();
    let profile = MockProfileService::default();
    let app = build_app(state_with(auth.clone(), github.clone(), profile.clone()));
    let body = Body::from_stream(stream::once(async {
        panic!("items polled request content");
        #[allow(unreachable_code)]
        Ok::<Bytes, Infallible>(Bytes::new())
    }));
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/v1/items").body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let method = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/items")
                .body(Body::from(vec![0; 1_000_001]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(method.headers()[header::ALLOW], "GET");
    assert_eq!(auth.call_count(), 0);
    assert_eq!(github.call_count(), 0);
    assert_eq!(profile.committed_write_count(), 0);
}
