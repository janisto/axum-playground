use std::sync::Arc;

use axum::{
    Router,
    extract::Request,
    http::{HeaderName, Method, StatusCode, header},
    middleware::{Next, from_fn},
    response::Response,
};
use axum_observability::ObservabilityLayer;
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
};

use crate::{
    http::{health, v1},
    middleware::{
        recover::panic_recovery_middleware, security::security_headers_middleware,
        timeout::timeout_middleware,
    },
    problem::problem_response,
    state::AppState,
    telemetry::observability_config,
};

const MAX_REQUEST_BODY_SIZE_BYTES: usize = 1024 * 1024;

pub fn build_app(state: Arc<AppState>) -> Router {
    build_app_with_routes(state, Router::new())
}

pub fn build_app_with_routes(state: Arc<AppState>, extra_routes: Router<Arc<AppState>>) -> Router {
    let cors_layer = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::ACCEPT,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-csrf-token"),
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("traceparent"),
            HeaderName::from_static("tracestate"),
        ])
        .expose_headers([
            header::ALLOW,
            header::LINK,
            header::LOCATION,
            header::RETRY_AFTER,
            header::WWW_AUTHENTICATE,
            HeaderName::from_static("x-ratelimit-reset"),
            HeaderName::from_static("x-request-id"),
        ])
        .max_age(std::time::Duration::from_secs(300));

    Router::new()
        .route("/health", axum::routing::get(health::health_handler))
        .route(
            "/schemas/ErrorModel.json",
            axum::routing::get(crate::http::schema::error_model_schema_handler),
        )
        .merge(v1::docs::ui_router())
        .nest("/v1", v1::router())
        .merge(extra_routes)
        .fallback(not_found_handler)
        .method_not_allowed_fallback(method_not_allowed_handler)
        .layer(
            ServiceBuilder::new()
                .layer(ObservabilityLayer::new(observability_config()))
                .layer(from_fn(panic_recovery_middleware))
                .layer(from_fn(security_headers_middleware))
                .layer(cors_layer)
                .layer(from_fn(timeout_middleware))
                .layer(from_fn(payload_too_large_problem_middleware))
                .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_SIZE_BYTES)),
        )
        .with_state(state)
}

async fn payload_too_large_problem_middleware(request: Request, next: Next) -> Response {
    let request_headers = request.headers().clone();
    let response = next.run(request).await;
    let has_problem_content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| matches!(value, "application/problem+json" | "application/cbor"));

    if response.status() == StatusCode::PAYLOAD_TOO_LARGE && !has_problem_content_type {
        problem_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body is too large",
            &request_headers,
        )
    } else {
        response
    }
}

async fn not_found_handler(request: Request) -> Response {
    problem_response(
        StatusCode::NOT_FOUND,
        "resource not found",
        request.headers(),
    )
}

async fn method_not_allowed_handler(request: Request) -> Response {
    problem_response(
        StatusCode::METHOD_NOT_ALLOWED,
        format!("method {} not allowed", request.method()),
        request.headers(),
    )
}
