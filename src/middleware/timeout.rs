use std::time::Duration;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

use crate::problem::problem_response;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn timeout_middleware(request: Request, next: Next) -> Response {
    let request_headers = request.headers().clone();

    match tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => timeout_response(&request_headers),
    }
}

fn timeout_response(request_headers: &axum::http::HeaderMap) -> Response {
    problem_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "request timed out",
        request_headers,
    )
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::HeaderMap};

    use super::timeout_response;
    use crate::problem::ProblemDetails;

    #[tokio::test]
    async fn processing_timeout_is_a_temporary_server_failure() {
        let response = timeout_response(&HeaderMap::new());

        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let body = to_bytes(response.into_body(), 4_096)
            .await
            .expect("problem body should be readable");
        let problem: ProblemDetails =
            serde_json::from_slice(&body).expect("problem body should deserialize");
        assert_eq!(problem.status, 503);
        assert_eq!(problem.detail.as_deref(), Some("request timed out"));
    }
}
