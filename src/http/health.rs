use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Service health", body = HealthResponse, content_type = "application/json")
    )
)]
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "healthy" })
}
