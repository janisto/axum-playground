use axum::{http::StatusCode, response::Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::http::{
    codec::{ResponseFormat, success_response},
    extract::NoQuery,
};
use crate::problem::ProblemResponse;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub status: &'static str,
}

#[utoipa::path(
    get,
    path = "/health",
    operation_id = "getHealth",
    security(()),
    responses(
        (status = 200, description = "Service liveness", content(
            (Health = "application/json"),
            (Health = "application/cbor")
        )),
        (status = 400, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn health_handler(format: ResponseFormat, _query: NoQuery) -> Response {
    success_response(StatusCode::OK, format, &Health { status: "healthy" })
}
