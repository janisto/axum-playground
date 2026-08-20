use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderValue, Method, header},
    middleware::{Next, from_fn},
    response::Response,
    routing::{MethodFilter, on},
};
use axum_observability::ObservabilityLayer;
use tower::ServiceBuilder;

use crate::{
    http::{codec::MAX_REQUEST_BODY_SIZE_BYTES, health, v1},
    middleware::{recover::panic_recovery_middleware, security::security_headers_middleware},
    problem::{ProblemCode, problem_response},
    state::AppState,
    telemetry::observability_config,
};

pub fn build_app(state: Arc<AppState>) -> Router {
    build_app_with_routes(state, Router::new())
}

pub fn build_app_with_routes(state: Arc<AppState>, extra_routes: Router<Arc<AppState>>) -> Router {
    Router::new()
        .route("/health", on(MethodFilter::GET, health::health_handler))
        .merge(v1::docs::router())
        .merge(v1::docs::ui_router())
        .nest("/v1", v1::router())
        .merge(extra_routes)
        .fallback(not_found_handler)
        .method_not_allowed_fallback(method_not_allowed_handler)
        .layer(
            ServiceBuilder::new()
                .layer(ObservabilityLayer::new(observability_config()))
                .layer(from_fn(security_headers_middleware))
                .layer(from_fn(panic_recovery_middleware))
                .layer(from_fn(portable_head_rejection_middleware))
                .layer(from_fn(declared_body_size_middleware)),
        )
        .with_state(state)
}

async fn portable_head_rejection_middleware(request: Request, next: Next) -> Response {
    if request.method() == Method::HEAD && portable_allow(request.uri().path()).is_some() {
        let mut response = method_not_allowed_handler(request).await;
        *response.body_mut() = Body::empty();
        return response;
    }
    next.run(request).await
}

async fn declared_body_size_middleware(request: Request, next: Next) -> Response {
    if is_portable_body_operation(request.method(), request.uri().path()) {
        let values = request
            .headers()
            .get_all(header::CONTENT_LENGTH)
            .iter()
            .collect::<Vec<_>>();
        if !values.is_empty() {
            let valid = (values.len() == 1).then(|| values[0]).and_then(|value| {
                value
                    .to_str()
                    .ok()
                    .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
                    .and_then(|value| value.parse::<u64>().ok())
            });
            let Some(length) = valid else {
                return problem_response(ProblemCode::InvalidRequest, request.headers());
            };
            if length > MAX_REQUEST_BODY_SIZE_BYTES as u64 {
                return problem_response(ProblemCode::PayloadTooLarge, request.headers());
            }
        }
    }
    next.run(request).await
}

fn is_portable_body_operation(method: &Method, path: &str) -> bool {
    matches!(
        (method, path),
        (&Method::POST, "/v1/hello" | "/v1/profile") | (&Method::PATCH, "/v1/profile")
    )
}

async fn not_found_handler(request: Request) -> Response {
    problem_response(ProblemCode::NotFound, request.headers())
}

async fn method_not_allowed_handler(request: Request) -> Response {
    let mut response = problem_response(ProblemCode::MethodNotAllowed, request.headers());
    if let Some(allowed) = portable_allow(request.uri().path()) {
        response.headers_mut().insert(header::ALLOW, allowed);
    }
    response
}

fn portable_allow(path: &str) -> Option<HeaderValue> {
    let value = match path {
        "/health" | "/v1/items" | "/openapi.json" => "GET",
        "/v1/hello" => "GET, POST",
        "/v1/profile" => "GET, POST, PATCH, DELETE",
        path if is_github_path(path) => "GET",
        _ => return None,
    };
    Some(HeaderValue::from_static(value))
}

fn is_github_path(path: &str) -> bool {
    let Some(path) = path.strip_prefix('/') else {
        return false;
    };
    let segments = path.split('/').collect::<Vec<_>>();
    matches!(
        segments.as_slice(),
        ["v1", "github", "owners", _]
            | ["v1", "github", "owners", _, "repos"]
            | ["v1", "github", "repos", _, _]
            | [
                "v1",
                "github",
                "repos",
                _,
                _,
                "activity" | "languages" | "tags"
            ]
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, header};

    use super::{is_github_path, is_portable_body_operation, portable_allow};

    #[test]
    fn limit_applies_only_after_a_supported_body_operation_is_selected() {
        assert!(is_portable_body_operation(&Method::POST, "/v1/hello"));
        assert!(is_portable_body_operation(&Method::POST, "/v1/profile"));
        assert!(is_portable_body_operation(&Method::PATCH, "/v1/profile"));
        assert!(!is_portable_body_operation(&Method::GET, "/v1/hello"));
        assert!(!is_portable_body_operation(&Method::PUT, "/v1/profile"));
        assert!(!is_portable_body_operation(&Method::POST, "/missing"));
    }

    #[test]
    fn allow_values_apply_only_to_exact_portable_path_shapes() {
        for path in [
            "/v1/github/owners/octocat",
            "/v1/github/owners/octocat/repos",
            "/v1/github/repos/octocat/hello-world",
            "/v1/github/repos/octocat/hello-world/activity",
            "/v1/github/repos/octocat/hello-world/languages",
            "/v1/github/repos/octocat/hello-world/tags",
        ] {
            assert!(is_github_path(path), "expected GitHub path: {path}");
            assert_eq!(
                portable_allow(path),
                Some(header::HeaderValue::from_static("GET"))
            );
        }

        for path in [
            "v1/github/owners/octocat",
            "//v1/github/owners/octocat",
            "/missing",
            "/v1/github",
            "/v1/github/owners",
            "/v1/github/owners/octocat/extra",
            "/v1/github/repos/octocat",
            "/v1/github/repos/octocat/hello-world/extra",
        ] {
            assert!(!is_github_path(path), "unexpected GitHub path: {path}");
            assert_eq!(portable_allow(path), None);
        }
    }
}
