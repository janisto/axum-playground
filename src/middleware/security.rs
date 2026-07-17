use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};

pub async fn security_headers_middleware(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    set_header_if_missing(
        headers,
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    set_header_if_missing(
        headers,
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static("frame-ancestors 'none'"),
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

fn set_header_if_missing(headers: &mut HeaderMap, name: HeaderName, value: HeaderValue) {
    if !headers.contains_key(&name) {
        headers.insert(name, value);
    }
}
