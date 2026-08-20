use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{MethodFilter, on},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    http::{
        codec::{ResponseFormat, success_response_with_headers},
        extract::StrictQuery,
    },
    pagination::{
        cursor::{CursorScope, decode_cursor},
        paginate, resolve_limit,
    },
    problem::{ProblemCode, ProblemResponse, problem_response},
    state::AppState,
};

const DEFAULT_LIMIT: u16 = 20;
const MAX_LIMIT: u16 = 100;
const ALLOWED_CATEGORIES: &[&str] = &[
    "electronics",
    "tools",
    "accessories",
    "robotics",
    "power",
    "components",
];

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Money {
    #[serde(rename = "amountMinor")]
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub amount_minor: u64,
    pub currency: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub category: String,
    pub price: Money,
    #[serde(rename = "inStock")]
    pub in_stock: bool,
    #[serde(rename = "createdAt")]
    #[schema(format = DateTime)]
    pub created_at: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemPage {
    pub items: Vec<Item>,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub total: u64,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/items", on(MethodFilter::GET, list_items_handler))
}

#[utoipa::path(
    get,
    path = "/v1/items",
    operation_id = "listItems",
    tag = "Items",
    security(()),
    params(
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor; the query is closed", max_length = 2048),
        ("limit" = Option<u16>, Query, description = "Maximum items per page; the query is closed", minimum = 1, maximum = 100),
        ("category" = Option<String>, Query, description = "Filter by category")
    ),
    responses(
        (status = 200, description = "Paginated items", headers(("Link" = String, description = "RFC 8288 pagination links")), content(
            (ItemPage = "application/json"),
            (ItemPage = "application/cbor")
        )),
        (status = 400, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn list_items_handler(
    format: ResponseFormat,
    headers: HeaderMap,
    query: StrictQuery,
) -> Response {
    let Ok(query) = query.closed(&["cursor", "limit", "category"]) else {
        return problem_response(ProblemCode::InvalidRequest, &headers);
    };
    let Some(limit) = resolve_limit(query.get("limit"), DEFAULT_LIMIT, MAX_LIMIT) else {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    };

    let category = query.get("category");
    if let Some(category) = category
        && !ALLOWED_CATEGORIES.contains(&category)
    {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }

    let cursor = match query.get("cursor") {
        None => None,
        Some(value) => match decode_cursor(value) {
            Ok(cursor) => Some(cursor),
            Err(_) => return problem_response(ProblemCode::InvalidRequest, &headers),
        },
    };

    let filtered_items = all_items()
        .into_iter()
        .filter(|item| category.is_none_or(|category| item.category == category))
        .collect::<Vec<_>>();

    let scope = CursorScope {
        operation: "listItems",
        owner: None,
        repository: None,
        limit,
        category,
    };
    let query_pairs = category
        .iter()
        .map(|category| ("category".to_owned(), (*category).to_owned()))
        .collect::<Vec<_>>();
    let Ok(page) = paginate(
        &filtered_items,
        cursor.as_ref(),
        &scope,
        |item| item.id.as_str(),
        "/v1/items",
        &query_pairs,
    ) else {
        return problem_response(ProblemCode::InvalidRequest, &headers);
    };

    let extra_headers = (!page.link_header.is_empty())
        .then(|| {
            HeaderValue::from_str(&page.link_header).expect("link header should be valid ASCII")
        })
        .map(|value| vec![(header::LINK, value)])
        .unwrap_or_default();

    success_response_with_headers(
        StatusCode::OK,
        format,
        &ItemPage {
            items: page.items,
            total: page.total as u64,
        },
        extra_headers,
    )
}

fn all_items() -> Vec<Item> {
    vec![
        item(
            "item-001",
            "Alpha Widget",
            "electronics",
            2999,
            true,
            "2024-01-15T10:30:00.000Z",
            "A versatile electronic widget for everyday use",
        ),
        item(
            "item-002",
            "Beta Gadget",
            "electronics",
            4999,
            true,
            "2024-01-16T11:00:00.000Z",
            "Advanced gadget with smart features",
        ),
        item(
            "item-003",
            "Gamma Tool",
            "tools",
            1550,
            false,
            "2024-01-17T09:15:00.000Z",
            "Precision tool for professional work",
        ),
        item(
            "item-004",
            "Delta Component",
            "electronics",
            899,
            true,
            "2024-01-18T14:45:00.000Z",
            "Essential component for electronics projects",
        ),
        item(
            "item-005",
            "Epsilon Sensor",
            "electronics",
            3499,
            true,
            "2024-01-19T08:00:00.000Z",
            "High-precision environmental sensor",
        ),
        item(
            "item-006",
            "Zeta Cable",
            "accessories",
            1299,
            true,
            "2024-01-20T16:30:00.000Z",
            "Premium quality data cable",
        ),
        item(
            "item-007",
            "Eta Adapter",
            "accessories",
            999,
            false,
            "2024-01-21T10:00:00.000Z",
            "Universal power adapter",
        ),
        item(
            "item-008",
            "Theta Board",
            "electronics",
            8999,
            true,
            "2024-01-22T11:30:00.000Z",
            "Development board for prototyping",
        ),
        item(
            "item-009",
            "Iota Switch",
            "electronics",
            599,
            true,
            "2024-01-23T09:45:00.000Z",
            "Tactile push button switch",
        ),
        item(
            "item-010",
            "Kappa Display",
            "electronics",
            4599,
            true,
            "2024-01-24T13:00:00.000Z",
            "OLED display module",
        ),
        item(
            "item-011",
            "Lambda Motor",
            "robotics",
            2499,
            true,
            "2024-01-25T08:30:00.000Z",
            "DC motor for robotics projects",
        ),
        item(
            "item-012",
            "Mu Servo",
            "robotics",
            1899,
            false,
            "2024-01-26T15:00:00.000Z",
            "High-torque servo motor",
        ),
        item(
            "item-013",
            "Nu Battery",
            "power",
            1499,
            true,
            "2024-01-27T10:15:00.000Z",
            "Rechargeable lithium battery pack",
        ),
        item(
            "item-014",
            "Xi Charger",
            "power",
            2299,
            true,
            "2024-01-28T11:45:00.000Z",
            "Smart battery charger",
        ),
        item(
            "item-015",
            "Omicron Relay",
            "electronics",
            799,
            true,
            "2024-01-29T09:00:00.000Z",
            "5V relay module",
        ),
        item(
            "item-016",
            "Pi Controller",
            "electronics",
            5599,
            true,
            "2024-01-30T14:30:00.000Z",
            "Microcontroller board",
        ),
        item(
            "item-017",
            "Rho Resistor Kit",
            "components",
            1199,
            true,
            "2024-02-01T08:00:00.000Z",
            "Assorted resistor pack",
        ),
        item(
            "item-018",
            "Sigma Capacitor Set",
            "components",
            1399,
            true,
            "2024-02-02T10:30:00.000Z",
            "Electrolytic capacitor assortment",
        ),
        item(
            "item-019",
            "Tau LED Pack",
            "components",
            699,
            true,
            "2024-02-03T11:00:00.000Z",
            "Multi-color LED assortment",
        ),
        item(
            "item-020",
            "Upsilon Wire Set",
            "accessories",
            899,
            false,
            "2024-02-04T09:15:00.000Z",
            "Jumper wire kit",
        ),
        item(
            "item-021",
            "Phi Breadboard",
            "tools",
            499,
            true,
            "2024-02-05T13:45:00.000Z",
            "Solderless breadboard",
        ),
        item(
            "item-022",
            "Chi Soldering Iron",
            "tools",
            3599,
            true,
            "2024-02-06T10:00:00.000Z",
            "Temperature-controlled soldering station",
        ),
        item(
            "item-023",
            "Psi Multimeter",
            "tools",
            4299,
            true,
            "2024-02-07T11:30:00.000Z",
            "Digital multimeter with auto-ranging",
        ),
        item(
            "item-024",
            "Omega Oscilloscope",
            "tools",
            29999,
            true,
            "2024-02-08T14:00:00.000Z",
            "Portable digital oscilloscope",
        ),
        item(
            "item-025",
            "Alpha Pro Widget",
            "electronics",
            5999,
            true,
            "2024-02-09T08:30:00.000Z",
            "Professional-grade widget with extended features",
        ),
        item(
            "item-026",
            "Beta Max Gadget",
            "electronics",
            7999,
            false,
            "2024-02-10T09:00:00.000Z",
            "Maximum performance gadget",
        ),
        item(
            "item-027",
            "Gamma Plus Tool",
            "tools",
            2599,
            true,
            "2024-02-11T10:15:00.000Z",
            "Enhanced precision tool",
        ),
        item(
            "item-028",
            "Delta Ultra Component",
            "electronics",
            1699,
            true,
            "2024-02-12T11:45:00.000Z",
            "Ultra-reliable component",
        ),
        item(
            "item-029",
            "Epsilon HD Sensor",
            "electronics",
            5499,
            true,
            "2024-02-13T13:00:00.000Z",
            "High-definition sensor array",
        ),
        item(
            "item-030",
            "Zeta Premium Cable",
            "accessories",
            1999,
            true,
            "2024-02-14T15:30:00.000Z",
            "Gold-plated premium cable",
        ),
    ]
}

fn item(
    id: &str,
    name: &str,
    category: &str,
    amount_minor: u64,
    in_stock: bool,
    created_at: &str,
    description: &str,
) -> Item {
    Item {
        id: id.to_owned(),
        name: name.to_owned(),
        category: category.to_owned(),
        price: Money {
            amount_minor,
            currency: "USD".to_owned(),
        },
        in_stock,
        created_at: created_at.to_owned(),
        description: description.to_owned(),
    }
}
