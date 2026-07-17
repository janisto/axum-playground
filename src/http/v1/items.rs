use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    http::{
        codec::{ResponseFormat, success_response_with_headers},
        extract::ProblemQuery,
    },
    pagination::{cursor::decode_cursor, paginate},
    problem::{ProblemResponse, problem_response},
    state::AppState,
};

const ITEM_CURSOR_KIND: &str = "item";
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: i64 = 100;
const ALLOWED_CATEGORIES: &[&str] = &[
    "electronics",
    "tools",
    "accessories",
    "robotics",
    "power",
    "components",
];

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub category: String,
    pub price: f64,
    #[serde(rename = "inStock")]
    pub in_stock: bool,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct ItemsListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub category: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ItemsListData {
    pub items: Vec<Item>,
    pub total: usize,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/items", get(list_items_handler))
}

#[utoipa::path(
    get,
    path = "/v1/items",
    tag = "Items",
    params(
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor from previous response"),
        ("limit" = Option<i64>, Query, description = "Maximum items per page", minimum = 1, maximum = 100),
        ("category" = Option<String>, Query, description = "Filter by category")
    ),
    responses(
        (status = 200, description = "Paginated items", headers(("Link" = String, description = "RFC 8288 pagination links")), content(
            (ItemsListData = "application/json"),
            (ItemsListData = "application/cbor")
        )),
        (status = 400, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse)
    )
)]
pub async fn list_items_handler(
    format: ResponseFormat,
    headers: HeaderMap,
    ProblemQuery(query): ProblemQuery<ItemsListQuery>,
) -> Response {
    if let Some(limit) = query.limit
        && (limit <= 0 || limit > MAX_LIMIT)
    {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
    }

    if let Some(category) = query.category.as_deref()
        && !ALLOWED_CATEGORIES.contains(&category)
    {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
    }

    let Ok(cursor) = decode_cursor(query.cursor.as_deref().unwrap_or_default()) else {
        return problem_response(StatusCode::BAD_REQUEST, "invalid cursor format", &headers);
    };

    if !cursor.kind.is_empty() && cursor.kind != ITEM_CURSOR_KIND {
        return problem_response(StatusCode::BAD_REQUEST, "cursor type mismatch", &headers);
    }

    let filtered_items = all_items()
        .into_iter()
        .filter(|item| {
            query
                .category
                .as_deref()
                .is_none_or(|category| item.category == category)
        })
        .collect::<Vec<_>>();

    if !cursor.value.is_empty() && !filtered_items.iter().any(|item| item.id == cursor.value) {
        return problem_response(
            StatusCode::BAD_REQUEST,
            "cursor references unknown item",
            &headers,
        );
    }

    let limit = default_limit(query.limit);
    let query_pairs = query
        .category
        .iter()
        .map(|category| ("category".to_owned(), category.clone()))
        .collect::<Vec<_>>();
    let page = paginate(
        &filtered_items,
        &cursor,
        limit,
        ITEM_CURSOR_KIND,
        |item| item.id.as_str(),
        "/v1/items",
        &query_pairs,
    );

    let extra_headers = (!page.link_header.is_empty())
        .then(|| {
            HeaderValue::from_str(&page.link_header).expect("link header should be valid ASCII")
        })
        .map(|value| vec![(header::LINK, value)])
        .unwrap_or_default();

    success_response_with_headers(
        StatusCode::OK,
        format,
        &ItemsListData {
            items: page.items,
            total: page.total,
        },
        extra_headers,
    )
}

fn default_limit(limit: Option<i64>) -> usize {
    match limit {
        Some(limit) if limit > 0 => limit as usize,
        _ => DEFAULT_LIMIT,
    }
}

fn all_items() -> Vec<Item> {
    vec![
        item(
            "item-001",
            "Alpha Widget",
            "electronics",
            29.99,
            true,
            "2024-01-15T10:30:00Z",
            "A versatile electronic widget for everyday use",
        ),
        item(
            "item-002",
            "Beta Gadget",
            "electronics",
            49.99,
            true,
            "2024-01-16T11:00:00Z",
            "Advanced gadget with smart features",
        ),
        item(
            "item-003",
            "Gamma Tool",
            "tools",
            15.50,
            false,
            "2024-01-17T09:15:00Z",
            "Precision tool for professional work",
        ),
        item(
            "item-004",
            "Delta Component",
            "electronics",
            8.99,
            true,
            "2024-01-18T14:45:00Z",
            "Essential component for electronics projects",
        ),
        item(
            "item-005",
            "Epsilon Sensor",
            "electronics",
            34.99,
            true,
            "2024-01-19T08:00:00Z",
            "High-precision environmental sensor",
        ),
        item(
            "item-006",
            "Zeta Cable",
            "accessories",
            12.99,
            true,
            "2024-01-20T16:30:00Z",
            "Premium quality data cable",
        ),
        item(
            "item-007",
            "Eta Adapter",
            "accessories",
            9.99,
            false,
            "2024-01-21T10:00:00Z",
            "Universal power adapter",
        ),
        item(
            "item-008",
            "Theta Board",
            "electronics",
            89.99,
            true,
            "2024-01-22T11:30:00Z",
            "Development board for prototyping",
        ),
        item(
            "item-009",
            "Iota Switch",
            "electronics",
            5.99,
            true,
            "2024-01-23T09:45:00Z",
            "Tactile push button switch",
        ),
        item(
            "item-010",
            "Kappa Display",
            "electronics",
            45.99,
            true,
            "2024-01-24T13:00:00Z",
            "OLED display module",
        ),
        item(
            "item-011",
            "Lambda Motor",
            "robotics",
            24.99,
            true,
            "2024-01-25T08:30:00Z",
            "DC motor for robotics projects",
        ),
        item(
            "item-012",
            "Mu Servo",
            "robotics",
            18.99,
            false,
            "2024-01-26T15:00:00Z",
            "High-torque servo motor",
        ),
        item(
            "item-013",
            "Nu Battery",
            "power",
            14.99,
            true,
            "2024-01-27T10:15:00Z",
            "Rechargeable lithium battery pack",
        ),
        item(
            "item-014",
            "Xi Charger",
            "power",
            22.99,
            true,
            "2024-01-28T11:45:00Z",
            "Smart battery charger",
        ),
        item(
            "item-015",
            "Omicron Relay",
            "electronics",
            7.99,
            true,
            "2024-01-29T09:00:00Z",
            "5V relay module",
        ),
        item(
            "item-016",
            "Pi Controller",
            "electronics",
            55.99,
            true,
            "2024-01-30T14:30:00Z",
            "Microcontroller board",
        ),
        item(
            "item-017",
            "Rho Resistor Kit",
            "components",
            11.99,
            true,
            "2024-02-01T08:00:00Z",
            "Assorted resistor pack",
        ),
        item(
            "item-018",
            "Sigma Capacitor Set",
            "components",
            13.99,
            true,
            "2024-02-02T10:30:00Z",
            "Electrolytic capacitor assortment",
        ),
        item(
            "item-019",
            "Tau LED Pack",
            "components",
            6.99,
            true,
            "2024-02-03T11:00:00Z",
            "Multi-color LED assortment",
        ),
        item(
            "item-020",
            "Upsilon Wire Set",
            "accessories",
            8.99,
            false,
            "2024-02-04T09:15:00Z",
            "Jumper wire kit",
        ),
        item(
            "item-021",
            "Phi Breadboard",
            "tools",
            4.99,
            true,
            "2024-02-05T13:45:00Z",
            "Solderless breadboard",
        ),
        item(
            "item-022",
            "Chi Soldering Iron",
            "tools",
            35.99,
            true,
            "2024-02-06T10:00:00Z",
            "Temperature-controlled soldering station",
        ),
        item(
            "item-023",
            "Psi Multimeter",
            "tools",
            42.99,
            true,
            "2024-02-07T11:30:00Z",
            "Digital multimeter with auto-ranging",
        ),
        item(
            "item-024",
            "Omega Oscilloscope",
            "tools",
            299.99,
            true,
            "2024-02-08T14:00:00Z",
            "Portable digital oscilloscope",
        ),
        item(
            "item-025",
            "Alpha Pro Widget",
            "electronics",
            59.99,
            true,
            "2024-02-09T08:30:00Z",
            "Professional-grade widget with extended features",
        ),
        item(
            "item-026",
            "Beta Max Gadget",
            "electronics",
            79.99,
            false,
            "2024-02-10T09:00:00Z",
            "Maximum performance gadget",
        ),
        item(
            "item-027",
            "Gamma Plus Tool",
            "tools",
            25.99,
            true,
            "2024-02-11T10:15:00Z",
            "Enhanced precision tool",
        ),
        item(
            "item-028",
            "Delta Ultra Component",
            "electronics",
            16.99,
            true,
            "2024-02-12T11:45:00Z",
            "Ultra-reliable component",
        ),
        item(
            "item-029",
            "Epsilon HD Sensor",
            "electronics",
            54.99,
            true,
            "2024-02-13T13:00:00Z",
            "High-definition sensor array",
        ),
        item(
            "item-030",
            "Zeta Premium Cable",
            "accessories",
            19.99,
            true,
            "2024-02-14T15:30:00Z",
            "Gold-plated premium cable",
        ),
    ]
}

fn item(
    id: &str,
    name: &str,
    category: &str,
    price: f64,
    in_stock: bool,
    created_at: &str,
    description: &str,
) -> Item {
    Item {
        id: id.to_owned(),
        name: name.to_owned(),
        category: category.to_owned(),
        price,
        in_stock,
        created_at: created_at.to_owned(),
        description: description.to_owned(),
    }
}
