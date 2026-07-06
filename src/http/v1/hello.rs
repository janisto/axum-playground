use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    http::codec::{decode_request_body, success_response},
    problem::problem_response,
    state::AppState,
    validation::valid_name,
};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct HelloData {
    pub message: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct HelloCreateBody {
    pub name: String,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/hello", get(get_hello_handler).post(create_hello_handler))
}

#[utoipa::path(
    get,
    path = "/v1/hello",
    tag = "Hello",
    responses(
        (status = 200, description = "Default greeting", content(
            (HelloData = "application/json"),
            (HelloData = "application/cbor")
        ))
    )
)]
pub async fn get_hello_handler(headers: HeaderMap) -> Response {
    success_response(
        StatusCode::OK,
        &headers,
        &HelloData {
            message: "Hello, World!".to_string(),
        },
    )
}

#[utoipa::path(
    post,
    path = "/v1/hello",
    tag = "Hello",
    request_body = HelloCreateBody,
    responses(
        (status = 201, description = "Personalized greeting", content(
            (HelloData = "application/json"),
            (HelloData = "application/cbor")
        )),
        (status = 422, description = "Validation failure")
    )
)]
pub async fn create_hello_handler(headers: HeaderMap, body: Bytes) -> Response {
    let input = match decode_request_body::<HelloCreateBody>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };

    if !valid_name(&input.name) {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
    }

    success_response(
        StatusCode::CREATED,
        &headers,
        &HelloData {
            message: format!("Hello, {}!", input.name),
        },
    )
}
