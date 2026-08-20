use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};

const STRICT_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; frame-ancestors 'none'";
const SWAGGER_UI_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; base-uri 'none'; connect-src 'self'; font-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'";

pub async fn security_headers_middleware(request: Request, next: Next) -> Response {
    let request_path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let content_security_policy = if is_swagger_ui_document(&request_path, response.headers()) {
        SWAGGER_UI_CONTENT_SECURITY_POLICY
    } else {
        STRICT_CONTENT_SECURITY_POLICY
    };
    let headers = response.headers_mut();
    set_header_if_missing(
        headers,
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    set_header_if_missing(
        headers,
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(content_security_policy),
    );
    set_header_if_missing(
        headers,
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    set_header_if_missing(
        headers,
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    set_header_if_missing(
        headers,
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()",
        ),
    );
    set_header_if_missing(
        headers,
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    set_header_if_missing(
        headers,
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    set_header_if_missing(
        headers,
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );

    response
}

fn is_swagger_ui_document(path: &str, headers: &HeaderMap) -> bool {
    path.starts_with("/api-docs/")
        && headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/html"))
}

fn set_header_if_missing(headers: &mut HeaderMap, name: HeaderName, value: HeaderValue) {
    if !headers.contains_key(&name) {
        headers.insert(name, value);
    }
}
