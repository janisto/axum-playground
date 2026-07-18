use std::{backtrace::Backtrace, panic::AssertUnwindSafe};

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use futures_util::FutureExt;

use crate::problem::problem_response;

pub async fn panic_recovery_middleware(request: Request, next: Next) -> Response {
    let request_headers = request.headers().clone();

    if let Ok(response) = AssertUnwindSafe(next.run(request)).catch_unwind().await {
        response
    } else {
        let backtrace = Backtrace::force_capture().to_string();
        tracing::error!(backtrace, "request panicked");

        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error",
            &request_headers,
        )
    }
}
