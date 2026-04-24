use std::{any::Any, backtrace::Backtrace, panic::AssertUnwindSafe};

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use futures_util::FutureExt;

use crate::problem::problem_response;

pub async fn panic_recovery_middleware(request: Request, next: Next) -> Response {
    let request_headers = request.headers().clone();

    match AssertUnwindSafe(next.run(request)).catch_unwind().await {
        Ok(response) => response,
        Err(payload) => {
            let panic_message = panic_payload_message(payload.as_ref());
            let backtrace = Backtrace::force_capture().to_string();

            match panic_message {
                Some(panic_message) => {
                    tracing::error!(panic_message, backtrace, "request panicked")
                }
                None => tracing::error!(backtrace, "request panicked"),
            }

            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error",
                &request_headers,
            )
        }
    }
}

fn panic_payload_message(payload: &(dyn Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}
