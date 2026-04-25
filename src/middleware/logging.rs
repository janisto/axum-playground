use std::{sync::Arc, time::Instant};

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::{middleware::request_id::RequestId, state::AppState};

#[derive(Clone, Debug, Eq, PartialEq)]
struct TraceContext {
    trace_id: String,
    span_id: String,
}

pub async fn request_logging_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let trace_context = request
        .headers()
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_traceparent);
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|request_id| request_id.as_str().to_string());
    let started_at = Instant::now();

    let response = next.run(request).await;

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    log_request(
        &state,
        method.as_ref(),
        &path,
        status,
        duration_ms,
        request_id.as_deref(),
        trace_context.as_ref(),
    );

    response
}

fn log_request(
    state: &AppState,
    method: &str,
    path: &str,
    status: u16,
    duration_ms: u64,
    request_id: Option<&str>,
    trace_context: Option<&TraceContext>,
) {
    let google_cloud_trace = trace_context.and_then(|trace_context| {
        state
            .config
            .resolved_google_project_id()
            .map(|project_id| format!("projects/{project_id}/traces/{}", trace_context.trace_id))
    });

    match (
        status >= 500,
        trace_context,
        request_id,
        google_cloud_trace.as_deref(),
    ) {
        (true, Some(trace_context), Some(request_id), Some(google_cloud_trace)) => tracing::error!(
            method,
            path,
            status,
            duration_ms,
            request_id,
            trace_id = %trace_context.trace_id,
            span_id = %trace_context.span_id,
            google_cloud_trace,
            "request completed"
        ),
        (true, Some(trace_context), Some(request_id), None) => tracing::error!(
            method,
            path,
            status,
            duration_ms,
            request_id,
            trace_id = %trace_context.trace_id,
            span_id = %trace_context.span_id,
            "request completed"
        ),
        (true, _, Some(request_id), _) => tracing::error!(
            method,
            path,
            status,
            duration_ms,
            request_id,
            "request completed"
        ),
        (true, _, None, _) => {
            tracing::error!(method, path, status, duration_ms, "request completed")
        }
        (false, Some(trace_context), Some(request_id), Some(google_cloud_trace)) => tracing::info!(
            method,
            path,
            status,
            duration_ms,
            request_id,
            trace_id = %trace_context.trace_id,
            span_id = %trace_context.span_id,
            google_cloud_trace,
            "request completed"
        ),
        (false, Some(trace_context), Some(request_id), None) => tracing::info!(
            method,
            path,
            status,
            duration_ms,
            request_id,
            trace_id = %trace_context.trace_id,
            span_id = %trace_context.span_id,
            "request completed"
        ),
        (false, _, Some(request_id), _) => tracing::info!(
            method,
            path,
            status,
            duration_ms,
            request_id,
            "request completed"
        ),
        (false, _, None, _) => {
            tracing::info!(method, path, status, duration_ms, "request completed")
        }
    }
}

fn parse_traceparent(value: &str) -> Option<TraceContext> {
    let mut parts = value.trim().split('-');
    let version = parts.next()?;
    let trace_id = parts.next()?;
    let span_id = parts.next()?;
    let trace_flags = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    if !is_hex(version, 2)
        || !is_hex(trace_id, 32)
        || !is_hex(span_id, 16)
        || !is_hex(trace_flags, 2)
        || trace_id == "00000000000000000000000000000000"
        || span_id == "0000000000000000"
    {
        return None;
    }

    Some(TraceContext {
        trace_id: trace_id.to_string(),
        span_id: span_id.to_string(),
    })
}

fn is_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::parse_traceparent;

    #[test]
    fn parses_valid_traceparent_header() {
        let trace_context =
            parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
                .expect("traceparent should parse");

        assert_eq!(trace_context.trace_id, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(trace_context.span_id, "b7ad6b7169203331");
    }

    #[test]
    fn rejects_invalid_traceparent_header() {
        assert!(parse_traceparent("00-xyz-bad-01").is_none());
        assert!(
            parse_traceparent("00-00000000000000000000000000000000-b7ad6b7169203331-01").is_none()
        );
        assert!(
            parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01").is_none()
        );
    }
}
