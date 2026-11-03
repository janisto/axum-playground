use std::{backtrace::Backtrace, panic::AssertUnwindSafe};

use axum::{extract::Request, middleware::Next, response::Response};
use futures_util::FutureExt;

use crate::{http::discovery::problem_for_path, problem::ProblemCode};

pub async fn panic_recovery_middleware(request: Request, next: Next) -> Response {
    let request_headers = request.headers().clone();
    let path = request.uri().path().to_owned();

    if let Ok(response) = AssertUnwindSafe(next.run(request)).catch_unwind().await {
        response
    } else {
        let backtrace = Backtrace::force_capture().to_string();
        tracing::error!(backtrace, "request panicked");

        problem_for_path(ProblemCode::InternalError, &path, &request_headers)
    }
}
