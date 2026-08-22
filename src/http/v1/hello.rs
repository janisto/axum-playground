use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{MethodFilter, on},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    http::{
        codec::{BufferedBody, ResponseFormat, decode_request_body, success_response},
        extract::NoQuery,
    },
    problem::{ProblemCode, ProblemResponse, problem_response},
    state::AppState,
    validation::valid_bounded_name,
};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Greeting {
    pub message: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HelloCreate {
    #[schema(min_length = 1, max_length = 100)]
    pub name: String,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/hello",
        on(MethodFilter::GET, get_hello_handler).on(MethodFilter::POST, create_hello_handler),
    )
}

#[utoipa::path(
    get,
    path = "/v1/hello",
    operation_id = "getHello",
    tag = "Hello",
    security(()),
    responses(
        (status = 200, description = "Default greeting", content(
            (Greeting = "application/json"),
            (Greeting = "application/cbor")
        )),
        (status = 400, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn get_hello_handler(format: ResponseFormat, _query: NoQuery) -> Response {
    success_response(
        StatusCode::OK,
        format,
        &Greeting {
            message: "Hello, World!".to_owned(),
        },
    )
}

#[utoipa::path(
    post,
    path = "/v1/hello",
    operation_id = "createHello",
    tag = "Hello",
    security(()),
    request_body(content(
        (HelloCreate = "application/json"),
        (HelloCreate = "application/cbor")
    )),
    responses(
        (status = 200, description = "Personalized greeting", content(
            (Greeting = "application/json"),
            (Greeting = "application/cbor")
        )),
        (status = 400, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 413, response = ProblemResponse),
        (status = 415, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn create_hello_handler(
    format: ResponseFormat,
    _query: NoQuery,
    headers: HeaderMap,
    BufferedBody(body): BufferedBody,
) -> Response {
    let input = match decode_request_body::<HelloCreate>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };

    if !valid_bounded_name(&input.name) {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }

    success_response(
        StatusCode::OK,
        format,
        &Greeting {
            message: format!("Hello, {}!", input.name),
        },
    )
}
