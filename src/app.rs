use std::sync::Arc;

use axum::{
    Router,
    extract::Request,
    http::{HeaderName, Method, StatusCode, header},
    middleware::{from_fn, from_fn_with_state},
    response::Response,
};
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
};

use crate::{
    http::{health, v1},
    middleware::{
        logging::request_logging_middleware, recover::panic_recovery_middleware,
        request_id::request_id_middleware, security::security_headers_middleware,
        timeout::timeout_middleware,
    },
    problem::problem_response,
    state::AppState,
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
        ])
        .expose_headers([
            header::LINK,
            header::LOCATION,
            header::RETRY_AFTER,
            HeaderName::from_static("x-ratelimit-reset"),
            HeaderName::from_static("x-request-id"),
        ])
        .max_age(std::time::Duration::from_secs(300));

    Router::new()
        .route("/health", axum::routing::get(health::health_handler))
        .merge(v1::docs::ui_router())
        .nest("/v1", v1::router())
        .merge(extra_routes)
        .fallback(not_found_handler)
        .method_not_allowed_fallback(method_not_allowed_handler)
        .layer(
            ServiceBuilder::new()
                .layer(from_fn(request_id_middleware))
                .layer(from_fn(panic_recovery_middleware))
                .layer(from_fn_with_state(
                    state.clone(),
                    request_logging_middleware,
                ))
                .layer(from_fn(security_headers_middleware))
                .layer(cors_layer)
                .layer(from_fn(timeout_middleware))
                .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_SIZE_BYTES)),
        )
        .with_state(state)
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
