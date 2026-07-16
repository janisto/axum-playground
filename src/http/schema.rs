use axum::{
    Json,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

pub async fn error_model_schema_handler() -> Response {
    let mut response = Json(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "ProblemDetails",
        "type": "object",
        "properties": {
            "type": { "type": "string", "format": "uri-reference" },
            "title": { "type": "string" },
            "status": { "type": "integer", "minimum": 100, "maximum": 599 },
            "detail": { "type": "string" },
            "instance": { "type": "string", "format": "uri-reference" },
            "errors": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "message": { "type": "string" } },
                    "required": ["message"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["status"]
    }))
    .into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/schema+json"),
    );
    response
}
