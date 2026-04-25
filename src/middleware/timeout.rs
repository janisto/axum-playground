use std::time::Duration;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

use crate::problem::problem_response;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn timeout_middleware(request: Request, next: Next) -> Response {
    let request_headers = request.headers().clone();

    match tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => problem_response(
            StatusCode::REQUEST_TIMEOUT,
            "request timed out",
            &request_headers,
        ),
    }
}
