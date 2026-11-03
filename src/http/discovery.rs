use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
    routing::{MethodFilter, get, on},
};

use crate::{
    http::{extract::NoQuery, negotiation::accepts_document},
    problem::{ProblemCode, ensure_vary, problem_response},
    state::AppState,
};

const CATALOG_TYPE: &str =
    "application/linkset+json; profile=\"https://www.rfc-editor.org/info/rfc9727\"";
const CATALOG: &str = r#"{"linkset":[{"anchor":"/","service-desc":[{"href":"/openapi.json","type":"application/json"}],"service-doc":[{"href":"/llms.txt","type":"text/plain"},{"href":"/llms-full.txt","type":"text/plain"}],"status":[{"href":"/health","type":"application/json"}]}]}"#;
const INDEX: &str = include_str!("../../llms.txt");
const GUIDE: &str = include_str!("../../llms-full.txt");
const CATALOG_LINK: &str =
    "</.well-known/api-catalog>; rel=\"api-catalog\"; type=\"application/linkset+json\"";
const SCHEMA_LINK: &str = "</openapi.json>; rel=\"service-desc\"; type=\"application/json\"";
const INDEX_LINK: &str = "</llms.txt>; rel=\"describedby\"; type=\"text/plain\"";

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/.well-known/api-catalog", get(catalog_handler))
        .route("/llms.txt", on(MethodFilter::GET, index_handler))
        .route("/llms-full.txt", on(MethodFilter::GET, guide_handler))
}

async fn catalog_handler(headers: HeaderMap, _query: NoQuery) -> Response {
    document_response(
        &headers,
        "application/linkset+json",
        &[("profile", "https://www.rfc-editor.org/info/rfc9727")],
        CATALOG_TYPE,
        CATALOG,
    )
}

async fn index_handler(headers: HeaderMap, _query: NoQuery) -> Response {
    text_response(&headers, INDEX)
}

async fn guide_handler(headers: HeaderMap, _query: NoQuery) -> Response {
    text_response(&headers, GUIDE)
}

fn text_response(headers: &HeaderMap, body: &'static str) -> Response {
    document_response(
        headers,
        "text/plain",
        &[("charset", "utf-8")],
        "text/plain; charset=utf-8",
        body,
    )
}

fn document_response(
    headers: &HeaderMap,
    media_type: &str,
    parameters: &[(&str, &str)],
    content_type: &'static str,
    body: &'static str,
) -> Response {
    if !accepts_document(headers, media_type, parameters) {
        return problem_response(ProblemCode::NotAcceptable, &HeaderMap::new());
    }
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, body.len())
        .body(Body::from(body))
        .expect("fixed documentation response should build");
    ensure_vary(response.headers_mut(), ["Accept"]);
    response
}

#[must_use]
pub(crate) fn is_document_path(path: &str) -> bool {
    matches!(
        path,
        "/openapi.json" | "/.well-known/api-catalog" | "/llms.txt" | "/llms-full.txt"
    )
}

pub(crate) fn problem_for_path(code: ProblemCode, path: &str, headers: &HeaderMap) -> Response {
    if is_document_path(path) {
        problem_response(code, &HeaderMap::new())
    } else {
        problem_response(code, headers)
    }
}

pub(crate) async fn discovery_links_middleware(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    append_discovery_links(&path, response.status(), response.headers_mut());
    response
}

fn append_discovery_links(path: &str, status: StatusCode, headers: &mut HeaderMap) {
    if !(status.is_success() || status.is_redirection()) {
        return;
    }
    let links: &[&str] = match path {
        "/health" => &[CATALOG_LINK, SCHEMA_LINK],
        "/.well-known/api-catalog" => &[CATALOG_LINK, INDEX_LINK],
        "/openapi.json" | "/llms-full.txt" | "/api-docs" | "/api-docs/" => &[INDEX_LINK],
        _ => &[],
    };
    for link in links {
        headers.append(header::LINK, HeaderValue::from_static(link));
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};

    use super::{INDEX_LINK, append_discovery_links};

    #[test]
    fn discovery_preserves_an_existing_schema_describedby_target() {
        let existing = HeaderValue::from_static(
            "</schema.json>; rel=\"describedby\"; type=\"application/schema+json\"",
        );
        let mut headers = HeaderMap::new();
        headers.append(header::LINK, existing.clone());
        append_discovery_links("/openapi.json", StatusCode::OK, &mut headers);
        assert_eq!(
            headers
                .get_all(header::LINK)
                .iter()
                .map(|value| value.to_str().expect("text Link"))
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                existing.to_str().expect("existing Link"),
                INDEX_LINK
            ])
        );
    }

    #[test]
    fn failed_documentation_and_unrelated_routes_do_not_advertise_success_links() {
        for (path, status) in [
            ("/openapi.json", StatusCode::NOT_ACCEPTABLE),
            ("/health", StatusCode::BAD_REQUEST),
            ("/api-docs/", StatusCode::INTERNAL_SERVER_ERROR),
            ("/v1/items", StatusCode::OK),
        ] {
            let mut headers = HeaderMap::new();
            append_discovery_links(path, status, &mut headers);
            assert!(!headers.contains_key(header::LINK));
        }
    }
}
