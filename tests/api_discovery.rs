mod common;

use std::{collections::BTreeSet, convert::Infallible};

use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use axum_playground::{
    AuthError, GitHubUpstreamErrorKind, MockAuthVerifier, MockGitHubService, MockProfileService,
    ProfileBackendError, ProfileOperation, ProfileServiceError, build_app,
};
use futures_util::stream;
use serde_json::Value;
use tower::ServiceExt;

use common::{assert_common_headers, body_bytes, read_json_body, read_text_body, state_with};

const CATALOG_TYPE: &str =
    "application/linkset+json; profile=\"https://www.rfc-editor.org/info/rfc9727\"";
const DOCUMENTS: [&str; 4] = [
    "/openapi.json",
    "/.well-known/api-catalog",
    "/llms.txt",
    "/llms-full.txt",
];

type Links = BTreeSet<(String, String, String)>;

fn links(headers: &HeaderMap) -> Links {
    headers
        .get_all(header::LINK)
        .iter()
        .flat_map(|field| {
            field
                .to_str()
                .expect("text Link field")
                .split(',')
                .flat_map(|link| {
                    let (target, attributes) = link
                        .trim()
                        .strip_prefix('<')
                        .expect("URI reference")
                        .split_once('>')
                        .expect("closed URI reference");
                    let attributes = attributes
                        .split(';')
                        .filter_map(|attribute| attribute.trim().split_once('='))
                        .map(|(name, value)| (name.trim(), value.trim().trim_matches('"')))
                        .collect::<std::collections::BTreeMap<_, _>>();
                    attributes
                        .get("rel")
                        .expect("relation")
                        .split_ascii_whitespace()
                        .map(|relation| {
                            (
                                target.to_owned(),
                                relation.to_owned(),
                                attributes.get("type").expect("typed link").to_string(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn has_link(headers: &HeaderMap, target: &str, relation: &str, media_type: &str) {
    assert!(
        links(headers).contains(&(
            target.to_owned(),
            relation.to_owned(),
            media_type.to_owned()
        )),
        "missing {relation} link to {target} with {media_type}"
    );
}

async fn request(method: Method, target: &str, accept: &[&str]) -> axum::response::Response {
    let auth = MockAuthVerifier::test_user().with_error(AuthError::ServiceUnavailable);
    let github = MockGitHubService::demo().with_upstream_error(GitHubUpstreamErrorKind::Transport);
    let profile = MockProfileService::default().with_error(ProfileServiceError::Unavailable(
        ProfileBackendError::new(ProfileOperation::Get, std::io::Error::other("unavailable")),
    ));
    let state = state_with(auth.clone(), github.clone(), profile.clone());
    let mut builder = Request::builder()
        .method(method)
        .uri(target)
        .header("x-request-id", "discovery-test")
        .header(header::ORIGIN, "https://client.example")
        .header(
            header::AUTHORIZATION,
            "Bearer discovery-must-not-authenticate",
        )
        .header(header::CONTENT_LENGTH, "2000000");
    for value in accept {
        builder = builder.header(header::ACCEPT, *value);
    }
    let body = Body::from_stream(stream::once(async {
        panic!("documentation polled request content");
        #[allow(unreachable_code)]
        Ok::<Bytes, Infallible>(Bytes::new())
    }));
    let response = build_app(state)
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("response");
    assert_eq!(auth.call_count(), 0, "discovery called authentication");
    assert_eq!(github.call_count(), 0, "discovery called GitHub");
    for operation in [
        ProfileOperation::Create,
        ProfileOperation::Get,
        ProfileOperation::Update,
        ProfileOperation::Delete,
    ] {
        assert_eq!(
            profile.operation_count(operation),
            0,
            "discovery called persistence"
        );
    }
    assert_eq!(profile.committed_write_count(), 0);
    assert_common_headers(&response, !target.starts_with("/api-docs"));
    assert_eq!(response.headers()["x-request-id"], "discovery-test");
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
    response
}

#[tokio::test]
async fn catalog_bootstraps_all_required_targets_from_one_origin_without_dependencies() {
    let retrieval = reqwest::Url::parse("http://localhost:8080/.well-known/api-catalog")
        .expect("retrieval URL");
    let response = request(Method::GET, retrieval.path(), &["application/linkset+json"]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], CATALOG_TYPE);
    has_link(
        response.headers(),
        "/.well-known/api-catalog",
        "api-catalog",
        "application/linkset+json",
    );
    has_link(response.headers(), "/llms.txt", "describedby", "text/plain");
    let catalog: Value = read_json_body(response).await;
    assert_eq!(catalog.as_object().expect("catalog object").len(), 1);
    let contexts = catalog["linkset"].as_array().expect("link contexts");
    assert_eq!(contexts.len(), 1);
    let context = &contexts[0];
    assert_eq!(
        retrieval
            .join(context["anchor"].as_str().expect("anchor"))
            .expect("root anchor")
            .as_str(),
        "http://localhost:8080/"
    );
    let expected = [
        ("service-desc", vec![("/openapi.json", "application/json")]),
        (
            "service-doc",
            vec![
                ("/llms.txt", "text/plain"),
                ("/llms-full.txt", "text/plain"),
            ],
        ),
        ("status", vec![("/health", "application/json")]),
    ];
    for (relation, expected_targets) in expected {
        let targets = context[relation].as_array().expect("relation targets");
        assert_eq!(targets.len(), expected_targets.len());
        for (path, media_type) in expected_targets {
            let target = targets
                .iter()
                .find(|target| target["href"] == path)
                .expect("required target");
            assert_eq!(target["type"], media_type);
            let url = retrieval
                .join(target["href"].as_str().expect("href"))
                .expect("resolved target");
            assert_eq!(url.origin(), retrieval.origin());
            let response = request(Method::GET, url.path(), &[media_type]).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE]
                    .to_str()
                    .expect("content type")
                    .split(';')
                    .next(),
                Some(media_type)
            );
            if media_type == "application/json" {
                let _: Value = read_json_body(response).await;
            }
        }
    }
}

#[tokio::test]
async fn guides_are_the_exact_embedded_root_files_and_catalog_head_has_get_metadata() {
    for (path, expected) in [
        ("/llms.txt", include_bytes!("../llms.txt").as_slice()),
        (
            "/llms-full.txt",
            include_bytes!("../llms-full.txt").as_slice(),
        ),
    ] {
        let response = request(Method::GET, path, &["text/plain; charset=UTF-8"]).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH]
                .to_str()
                .expect("content length")
                .parse::<usize>()
                .expect("length"),
            expected.len()
        );
        assert_eq!(body_bytes(response).await.as_ref(), expected);
    }
    let get = request(Method::GET, "/.well-known/api-catalog", &[]).await;
    let length = get.headers()[header::CONTENT_LENGTH].clone();
    let get_links = links(get.headers());
    assert_eq!(body_bytes(get).await.len().to_string(), length);
    let head = request(Method::HEAD, "/.well-known/api-catalog", &[]).await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()[header::CONTENT_TYPE], CATALOG_TYPE);
    assert_eq!(head.headers()[header::CONTENT_LENGTH], length);
    assert_eq!(links(head.headers()), get_links);
    assert!(body_bytes(head).await.is_empty());
}

#[tokio::test]
async fn health_and_documentation_advertise_typed_discovery_for_every_representation() {
    for accept in ["application/json", "application/cbor"] {
        let response = request(Method::GET, "/health", &[accept]).await;
        assert_eq!(response.status(), StatusCode::OK);
        has_link(
            response.headers(),
            "/.well-known/api-catalog",
            "api-catalog",
            "application/linkset+json",
        );
        has_link(
            response.headers(),
            "/openapi.json",
            "service-desc",
            "application/json",
        );
    }
    for path in [
        "/openapi.json",
        "/llms-full.txt",
        "/.well-known/api-catalog",
        "/api-docs",
        "/api-docs/",
    ] {
        let response = request(Method::GET, path, &[]).await;
        assert!(
            response.status().is_success() || response.status().is_redirection(),
            "{path}"
        );
        has_link(response.headers(), "/llms.txt", "describedby", "text/plain");
        if path == "/openapi.json" {
            let schema: Value = read_json_body(response).await;
            assert_eq!(
                schema["paths"]["/health"]["get"]["responses"]["200"]["headers"]["Link"]["schema"]
                    ["type"],
                "string"
            );
        }
    }
    let initializer = request(Method::GET, "/api-docs/swagger-initializer.js", &[]).await;
    assert_eq!(initializer.status(), StatusCode::OK);
    assert!(read_text_body(initializer).await.contains("/openapi.json"));
}

#[tokio::test]
async fn fixed_document_negotiation_honors_parameters_exclusions_and_repeated_accept_fields() {
    let cases: &[(&str, &[&str], StatusCode)] = &[
        (
            "/.well-known/api-catalog",
            &[""],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["application/json"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json; profile=\"https://www.rfc-editor.org/info/rfc9727\""],
            StatusCode::OK,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json; profile=\"https://www.rfc-editor.org/info/RFC9727\""],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json; profile=wrong"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json; unknown=value"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json; q=1; profile=ignored"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json; q=1; profile=\"https://www.rfc-editor.org/info/rfc9727\""],
            StatusCode::OK,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json; q=1; unknown=value"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["application/linkset+json;q=0, */*;q=1"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &["text/html", "application/linkset+json"],
            StatusCode::OK,
        ),
        (
            "/.well-known/api-catalog",
            &[
                "application/linkset+json; profile=\"https://www.rfc-editor.org/info/rfc9727\";q=0, application/linkset+json;q=1",
            ],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            &[
                "application/linkset+json;q=0;profile=\"https://www.rfc-editor.org/info/rfc9727\", application/linkset+json;q=1",
            ],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/llms.txt",
            &["text/plain; charset=\"UTF-8\""],
            StatusCode::OK,
        ),
        (
            "/llms.txt",
            &["text/plain; charset=latin1"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/llms.txt",
            &["text/plain; q=1; charset=latin1"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/llms.txt",
            &["text/plain; q=1; charset=UTF-8"],
            StatusCode::OK,
        ),
        (
            "/llms.txt",
            &["text/plain;q=0, text/*;q=1"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/llms.txt",
            &["text/plain;q=0;charset=utf-8, text/plain;q=1"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/llms.txt",
            &["text/plain; charset=utf-8; charset=UTF-8"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        ("/llms-full.txt", &["text/*"], StatusCode::OK),
        (
            "/llms-full.txt",
            &["application/cbor"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/openapi.json",
            &["application/json; charset=UTF-8"],
            StatusCode::OK,
        ),
        (
            "/openapi.json",
            &["application/json; q=1; charset=latin1"],
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/openapi.json",
            &["application/cbor"],
            StatusCode::NOT_ACCEPTABLE,
        ),
    ];
    for (path, accept, expected) in cases {
        let response = request(Method::GET, path, accept).await;
        assert_eq!(response.status(), *expected, "{path} {accept:?}");
        if *expected == StatusCode::NOT_ACCEPTABLE {
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/problem+json"
            );
            assert_eq!(
                read_json_body::<Value>(response).await["code"],
                "not_acceptable"
            );
        }
    }
}

#[tokio::test]
async fn documentation_queries_methods_and_head_errors_are_json_and_side_effect_free() {
    for path in DOCUMENTS {
        for query in [
            "?unknown=1",
            "?unknown=1&unknown=2",
            "?q=%",
            "?q=%FF",
            "?=1",
            "?x=1&&",
        ] {
            let response = request(Method::GET, &format!("{path}{query}"), &["application/cbor, application/json;q=0.5, application/linkset+json;q=0.5, text/plain;q=0.5"]).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}{query}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/problem+json"
            );
            assert_eq!(
                read_json_body::<Value>(response).await["code"],
                "invalid_request"
            );
        }
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
            Method::HEAD,
        ] {
            if path == "/.well-known/api-catalog" && method == Method::HEAD {
                continue;
            }
            let response = request(method.clone(), path, &["application/cbor"]).await;
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path}"
            );
            let allowed = response.headers()[header::ALLOW]
                .to_str()
                .expect("Allow")
                .split(',')
                .map(str::trim)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                allowed,
                if path == "/.well-known/api-catalog" {
                    BTreeSet::from(["GET", "HEAD"])
                } else {
                    BTreeSet::from(["GET"])
                }
            );
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/problem+json"
            );
            if method == Method::HEAD {
                assert!(body_bytes(response).await.is_empty());
            } else {
                assert_eq!(
                    read_json_body::<Value>(response).await["code"],
                    "method_not_allowed"
                );
            }
        }
    }
    for (target, accept, status) in [
        (
            "/.well-known/api-catalog?x=1",
            "application/linkset+json",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/.well-known/api-catalog",
            "application/cbor",
            StatusCode::NOT_ACCEPTABLE,
        ),
        (
            "/.well-known/api-catalog",
            "application/linkset+json;q=1;profile=wrong",
            StatusCode::NOT_ACCEPTABLE,
        ),
    ] {
        let response = request(Method::HEAD, target, &[accept]).await;
        assert_eq!(response.status(), status);
        let get = request(Method::GET, target, &[accept]).await;
        assert_eq!(
            response.headers()[header::CONTENT_LENGTH],
            get.headers()[header::CONTENT_LENGTH]
        );
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        assert!(body_bytes(response).await.is_empty());
    }
}

#[tokio::test]
async fn documentation_does_not_serve_other_repository_files_or_aliases() {
    for path in [
        "/.env",
        "/.git/config",
        "/Cargo.toml",
        "/README.md",
        "/src/http/discovery.rs",
        "/llms-full.txt/",
        "/.well-known/llms.txt",
        "/openapi.yaml",
    ] {
        let response = request(Method::GET, path, &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}
