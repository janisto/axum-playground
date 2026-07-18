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

            if let Some(panic_message) = panic_message {
                tracing::error!(panic_message, backtrace, "request panicked")
            } else {
                tracing::error!(backtrace, "request panicked")
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

#[cfg(test)]
mod tests {
    use super::panic_payload_message;

    #[test]
    fn panic_payload_message_extracts_only_string_payloads() {
        let owned = "owned panic".to_owned();

        assert_eq!(panic_payload_message(&"static panic"), Some("static panic"));
        assert_eq!(panic_payload_message(&owned), Some("owned panic"));
        assert_eq!(panic_payload_message(&42_u32), None);
    }
}
