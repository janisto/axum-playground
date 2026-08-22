mod common;

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{
    AuthError, GitHubServiceError, MockAuthVerifier, MockGitHubService, MockProfileService,
    ProfileBackendError, ProfileOperation, ProfileServiceError, build_app,
};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::{
    assert_common_headers, read_json_body, read_text_body, state_with, test_state,
};

const COMMON_RESPONSE_HEADERS: [&str; 5] = [
    "X-Request-ID",
    "Cache-Control",
    "X-Content-Type-Options",
    "X-Frame-Options",
    "Referrer-Policy",
];
const STRICT_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; frame-ancestors 'none'";
const SWAGGER_UI_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; base-uri 'none'; connect-src 'self'; font-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'";

struct OperationExpectation {
    path: &'static str,
    method: &'static str,
    operation_id: &'static str,
    statuses: &'static [&'static str],
    success: &'static str,
    protected: bool,
}

const OPERATIONS: [OperationExpectation; 14] = [
    operation(
        "/health",
        "get",
        "getHealth",
        &["200", "400", "406", "500"],
        "200",
        false,
    ),
    operation(
        "/v1/hello",
        "get",
        "getHello",
        &["200", "400", "406", "500"],
        "200",
        false,
    ),
    operation(
        "/v1/hello",
        "post",
        "createHello",
        &["200", "400", "406", "413", "415", "422", "500"],
        "200",
        false,
    ),
    operation(
        "/v1/items",
        "get",
        "listItems",
        &["200", "400", "406", "422", "500"],
        "200",
        false,
    ),
    operation(
        "/v1/profile",
        "post",
        "createProfile",
        &[
            "201", "400", "401", "406", "409", "413", "415", "422", "500", "503",
        ],
        "201",
        true,
    ),
    operation(
        "/v1/profile",
        "get",
        "getProfile",
        &["200", "400", "401", "404", "406", "500", "503"],
        "200",
        true,
    ),
    operation(
        "/v1/profile",
        "patch",
        "updateProfile",
        &[
            "200", "400", "401", "404", "406", "413", "415", "422", "500", "503",
        ],
        "200",
        true,
    ),
    operation(
        "/v1/profile",
        "delete",
        "deleteProfile",
        &["204", "400", "401", "404", "500", "503"],
        "204",
        true,
    ),
    github_operation("/v1/github/owners/{owner}", "getGitHubOwner"),
    github_operation(
        "/v1/github/owners/{owner}/repos",
        "listGitHubOwnerRepositories",
    ),
    github_operation("/v1/github/repos/{owner}/{repo}", "getGitHubRepository"),
    github_operation(
        "/v1/github/repos/{owner}/{repo}/activity",
        "listGitHubRepositoryActivity",
    ),
    github_operation(
        "/v1/github/repos/{owner}/{repo}/languages",
        "listGitHubRepositoryLanguages",
    ),
    github_operation(
        "/v1/github/repos/{owner}/{repo}/tags",
        "listGitHubRepositoryTags",
    ),
];

const fn operation(
    path: &'static str,
    method: &'static str,
    operation_id: &'static str,
    statuses: &'static [&'static str],
    success: &'static str,
    protected: bool,
) -> OperationExpectation {
    OperationExpectation {
        path,
        method,
        operation_id,
        statuses,
        success,
        protected,
    }
}

const fn github_operation(path: &'static str, operation_id: &'static str) -> OperationExpectation {
    operation(
        path,
        "get",
        operation_id,
        &[
            "200", "400", "404", "406", "422", "429", "500", "502", "504",
        ],
        "200",
        false,
    )
}

#[tokio::test]
async fn served_document_has_exact_inventory_security_statuses_and_media() {
    let document = served_document().await;
    assert_eq!(document["openapi"], "3.1.0");
    assert!(document["jsonSchemaDialect"].as_str().is_some());
    assert!(
        document["info"]["title"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(
        document["info"]["version"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );

    let actual_paths = document["paths"]
        .as_object()
        .expect("paths should be an object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_paths = OPERATIONS
        .iter()
        .map(|operation| operation.path)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_paths, expected_paths);

    let mut operation_ids = BTreeSet::new();
    for expected in &OPERATIONS {
        let operation = operation_at(&document, expected.path, expected.method);
        assert_eq!(operation["operationId"], expected.operation_id);
        assert!(operation_ids.insert(expected.operation_id));
        assert_eq!(
            operation["security"],
            if expected.protected {
                json!([{"bearerAuth":[]}])
            } else {
                json!([])
            }
        );

        let actual_statuses = operation["responses"]
            .as_object()
            .expect("responses should be an object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual_statuses,
            expected.statuses.iter().copied().collect(),
            "unexpected statuses for {} {}",
            expected.method,
            expected.path
        );
        assert!(!actual_statuses.contains("405"));

        for status in expected.statuses {
            let response = &operation["responses"][status];
            assert_response_headers(response, expected.method == "delete" && *status == "204");
            if *status == expected.success {
                if *status == "204" {
                    assert!(response.get("content").is_none());
                } else {
                    assert_eq!(
                        content_types(response),
                        set(&["application/json", "application/cbor"])
                    );
                }
            } else {
                assert_eq!(
                    content_types(response),
                    set(&["application/problem+json", "application/cbor"])
                );
                assert_exact_problem_schema(&document, expected.path, status, response);
            }
        }
    }

    assert_eq!(
        document["components"]["securitySchemes"]["bearerAuth"]["scheme"],
        "bearer"
    );
}

#[tokio::test]
async fn served_document_projects_exact_parameters_bodies_and_special_headers() {
    let document = served_document().await;
    let expected_parameters = BTreeMap::from([
        (("/health", "get"), vec!["X-Request-ID"]),
        (("/v1/hello", "get"), vec!["X-Request-ID"]),
        (("/v1/hello", "post"), vec!["X-Request-ID"]),
        (
            ("/v1/items", "get"),
            vec!["X-Request-ID", "category", "cursor", "limit"],
        ),
        (("/v1/profile", "post"), vec!["X-Request-ID"]),
        (("/v1/profile", "get"), vec!["X-Request-ID"]),
        (("/v1/profile", "patch"), vec!["X-Request-ID"]),
        (("/v1/profile", "delete"), vec!["X-Request-ID"]),
        (
            ("/v1/github/owners/{owner}", "get"),
            vec!["X-Request-ID", "owner"],
        ),
        (
            ("/v1/github/owners/{owner}/repos", "get"),
            vec!["X-Request-ID", "cursor", "limit", "owner"],
        ),
        (
            ("/v1/github/repos/{owner}/{repo}", "get"),
            vec!["X-Request-ID", "owner", "repo"],
        ),
        (
            ("/v1/github/repos/{owner}/{repo}/activity", "get"),
            vec!["X-Request-ID", "cursor", "limit", "owner", "repo"],
        ),
        (
            ("/v1/github/repos/{owner}/{repo}/languages", "get"),
            vec!["X-Request-ID", "owner", "repo"],
        ),
        (
            ("/v1/github/repos/{owner}/{repo}/tags", "get"),
            vec!["X-Request-ID", "cursor", "limit", "owner", "repo"],
        ),
    ]);

    for expected in &OPERATIONS {
        let operation = operation_at(&document, expected.path, expected.method);
        let mut parameters = operation["parameters"]
            .as_array()
            .expect("parameters should be an array")
            .iter()
            .map(|parameter| {
                (
                    parameter["name"].as_str().expect("parameter name"),
                    parameter,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            parameters.keys().copied().collect::<Vec<_>>(),
            expected_parameters[&(expected.path, expected.method)]
        );

        let request_id = parameters
            .remove("X-Request-ID")
            .expect("request ID parameter");
        assert_eq!(request_id["in"], "header");
        assert_eq!(request_id["required"], false);
        assert_eq!(request_id["schema"]["minLength"], 1);
        assert_eq!(request_id["schema"]["maxLength"], 128);
        assert_eq!(
            request_id["schema"]["pattern"],
            "^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"
        );
        assert!(
            request_id["description"]
                .as_str()
                .is_some_and(|description| {
                    ["missing", "invalid", "repeated", "comma-combined"]
                        .iter()
                        .all(|term| description.to_ascii_lowercase().contains(term))
                })
        );

        for (name, parameter) in parameters {
            if matches!(name, "owner" | "repo") {
                assert_eq!(parameter["in"], "path");
                assert_eq!(parameter["required"], true);
            } else {
                assert_eq!(parameter["in"], "query");
                assert_eq!(parameter["required"], false);
                assert!(
                    parameter["description"]
                        .as_str()
                        .is_some_and(|description| description
                            .to_ascii_lowercase()
                            .contains("closed"))
                );
            }
            assert_parameter_schema(name, &parameter["schema"]);
        }

        let body_expected = matches!(
            (expected.path, expected.method),
            ("/v1/hello", "post") | ("/v1/profile", "post" | "patch")
        );
        if body_expected {
            assert_eq!(operation["requestBody"]["required"], true);
            assert_eq!(
                operation["requestBody"]["content"]
                    .as_object()
                    .expect("request content")
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                set(&["application/json", "application/cbor"])
            );
        } else {
            assert!(operation.get("requestBody").is_none());
        }
    }

    assert_eq!(
        operation_at(&document, "/v1/profile", "post")["responses"]["201"]["headers"]["Location"]["schema"]
            ["const"],
        "/v1/profile"
    );
    for expected in &OPERATIONS {
        for status in expected.statuses {
            let method = expected.method;
            let path = expected.path;
            if (expected.path, expected.method, *status) != ("/v1/profile", "post", "201") {
                assert!(
                    operation_at(&document, expected.path, expected.method)["responses"][status]
                        ["headers"]
                        .get("Location")
                        .is_none(),
                    "unexpected Location on {method} {path} {status}"
                );
            }
        }
    }
    for method in ["post", "get", "patch", "delete"] {
        assert_eq!(
            operation_at(&document, "/v1/profile", method)["responses"]["401"]["headers"]["WWW-Authenticate"]
                ["schema"]["const"],
            "Bearer"
        );
    }
    for path in [
        "/v1/items",
        "/v1/github/owners/{owner}/repos",
        "/v1/github/repos/{owner}/{repo}/activity",
        "/v1/github/repos/{owner}/{repo}/tags",
    ] {
        assert!(
            operation_at(&document, path, "get")["responses"]["200"]["headers"]
                .get("Link")
                .is_some()
        );
    }
    for path in OPERATIONS
        .iter()
        .filter(|operation| operation.path.starts_with("/v1/github/"))
        .map(|operation| operation.path)
    {
        let headers = &operation_at(&document, path, "get")["responses"]["429"]["headers"];
        for name in ["Retry-After", "X-RateLimit-Reset"] {
            assert_eq!(headers[name]["schema"]["minimum"], 0);
            assert_eq!(
                headers[name]["schema"]["maximum"],
                9_007_199_254_740_991_u64
            );
        }
    }
}

#[tokio::test]
async fn served_document_dereferences_exact_portable_schema_shapes() {
    let document = served_document().await;

    let hello_create = request_schema(&document, "/v1/hello", "post", "application/json");
    assert_closed_object(&document, hello_create, &["name"], &["name"]);
    assert_bounded_name(property(&document, hello_create, "name"));
    let greeting = response_schema(&document, "/v1/hello", "get", "200", "application/json");
    assert_closed_object(&document, greeting, &["message"], &["message"]);

    let item_page = response_schema(&document, "/v1/items", "get", "200", "application/json");
    assert_closed_object(
        &document,
        item_page,
        &["items", "total"],
        &["items", "total"],
    );
    assert_eq!(property(&document, item_page, "items")["maxItems"], 100);
    let item = resolve_schema(&document, &property(&document, item_page, "items")["items"]);
    let item_fields = [
        "id",
        "name",
        "category",
        "price",
        "inStock",
        "createdAt",
        "description",
    ];
    assert_closed_object(&document, item, &item_fields, &item_fields);
    assert_eq!(
        property(&document, item, "id")["enum"]
            .as_array()
            .map(Vec::len),
        Some(30)
    );
    let money = property(&document, item, "price");
    assert_closed_object(
        &document,
        money,
        &["amountMinor", "currency"],
        &["amountMinor", "currency"],
    );
    assert_safe_integer(property(&document, money, "amountMinor"));
    assert_eq!(property(&document, money, "currency")["const"], "USD");

    let create = request_schema(&document, "/v1/profile", "post", "application/json");
    assert_closed_object(
        &document,
        create,
        &[
            "firstName",
            "lastName",
            "contactEmail",
            "phoneNumber",
            "marketingOptIn",
            "termsAccepted",
        ],
        &[
            "firstName",
            "lastName",
            "contactEmail",
            "phoneNumber",
            "termsAccepted",
        ],
    );
    assert_eq!(
        property(&document, create, "marketingOptIn")["default"],
        false
    );
    assert_eq!(property(&document, create, "termsAccepted")["const"], true);
    assert_input_contact_schemas(&document, create);

    let update = request_schema(&document, "/v1/profile", "patch", "application/json");
    assert_closed_object(
        &document,
        update,
        &[
            "firstName",
            "lastName",
            "contactEmail",
            "phoneNumber",
            "marketingOptIn",
        ],
        &[],
    );
    assert_eq!(resolve_schema(&document, update)["minProperties"], 1);
    for name in [
        "firstName",
        "lastName",
        "contactEmail",
        "phoneNumber",
        "marketingOptIn",
    ] {
        assert!(
            !allows_null(property(&document, update, name)),
            "{name} must be optional but non-null"
        );
    }
    assert_input_contact_schemas(&document, update);

    let profile = response_schema(&document, "/v1/profile", "get", "200", "application/json");
    let profile_fields = [
        "id",
        "firstName",
        "lastName",
        "contactEmail",
        "phoneNumber",
        "marketingOptIn",
        "termsAccepted",
        "createdAt",
        "updatedAt",
    ];
    assert_closed_object(&document, profile, &profile_fields, &profile_fields);
    assert_eq!(property(&document, profile, "id")["minLength"], 1);
    assert_eq!(property(&document, profile, "id")["maxLength"], 128);
    assert_eq!(property(&document, profile, "termsAccepted")["const"], true);
    let phone = property(&document, profile, "phoneNumber");
    assert_eq!(phone["type"], "string");
    assert_eq!(phone["minLength"], 8);
    assert_eq!(phone["maxLength"], 16);
    assert_eq!(phone["pattern"], "^\\+[1-9][0-9]{6,14}$");
    for name in ["createdAt", "updatedAt"] {
        assert_timestamp(property(&document, profile, name));
    }
    assert!(
        property(&document, profile, "contactEmail")["pattern"]
            .as_str()
            .is_some_and(|pattern| pattern.contains('`') && pattern.contains("@[a-z0-9]"))
    );

    let issue = &document["components"]["schemas"]["ProblemIssue"];
    assert_closed_object(&document, issue, &["detail", "source"], &["detail"]);
    let detail = property(&document, issue, "detail");
    assert_eq!(detail["minLength"], 1);
    assert_eq!(detail["maxLength"], 200);
    assert_eq!(
        resolve_schema(&document, &issue["properties"]["source"]),
        &document["components"]["schemas"]["ProblemSource"]
    );
    let source = &document["components"]["schemas"]["ProblemSource"];
    let variants = source["oneOf"].as_array().expect("source alternatives");
    assert_eq!(variants.len(), 3);
    for (variant, name) in variants.iter().zip(["pointer", "parameter", "header"]) {
        assert_closed_object(&document, variant, &[name], &[name]);
        let value = property(&document, variant, name);
        assert_eq!(value["minLength"], 1);
        assert_eq!(value["maxLength"], 256);
    }

    assert_github_schemas(&document);
    assert_no_nullable_keyword(&document);
}

#[tokio::test]
async fn discovery_is_local_dependency_free_and_all_references_resolve() {
    let auth = MockAuthVerifier::test_user().with_error(AuthError::ServiceUnavailable);
    let github = MockGitHubService::demo().with_error(GitHubServiceError::NotFound);
    let profile = MockProfileService::default().with_error(ProfileServiceError::Unavailable(
        ProfileBackendError::new(
            ProfileOperation::Get,
            std::io::Error::other("persistence secret"),
        ),
    ));
    let app = build_app(state_with(auth.clone(), github.clone(), profile.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert_common_headers(&response, true);
    let document: Value = read_json_body(response).await;

    let mut references = BTreeSet::new();
    collect_local_references(&document, &document, &mut references);
    assert!(!references.is_empty());
    assert_eq!(auth.call_count(), 0);
    assert_eq!(github.call_count(), 0);
    assert_eq!(profile.committed_write_count(), 0);
    for operation in [
        ProfileOperation::Create,
        ProfileOperation::Get,
        ProfileOperation::Update,
        ProfileOperation::Delete,
    ] {
        assert_eq!(profile.operation_count(operation), 0);
    }

    let serialized = serde_json::to_string(&document)
        .expect("document should serialize")
        .to_ascii_lowercase();
    for forbidden in [
        "github_token",
        "x-github-api-version",
        "\"authorization\"",
        "application/problem+cbor",
        "/schemas/errormodel.json",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "forbidden OpenAPI input or legacy surface: {forbidden}"
        );
    }
}

#[tokio::test]
async fn discovery_is_exposed_only_at_the_canonical_root_path() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .uri("/v1/openapi.json")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_common_headers(&response, true);
    let problem: Value = read_json_body(response).await;
    assert_eq!(problem["status"], 404);
    assert_eq!(problem["code"], "not_found");
}

#[tokio::test]
async fn swagger_ui_assets_use_a_scoped_content_security_policy() {
    let app = build_app(test_state());
    let redirect = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api-docs")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(redirect.status(), StatusCode::SEE_OTHER);
    assert_eq!(redirect.headers()[header::LOCATION], "/api-docs/");
    assert_eq!(
        redirect.headers()["content-security-policy"],
        STRICT_CONTENT_SECURITY_POLICY
    );

    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api-docs/")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(
        page.headers()["content-security-policy"],
        SWAGGER_UI_CONTENT_SECURITY_POLICY
    );
    assert!(
        page.headers()[header::CONTENT_TYPE]
            .to_str()
            .expect("content type")
            .starts_with("text/html")
    );
    let html = read_text_body(page).await;
    assert!(html.to_ascii_lowercase().contains("swagger ui"));
    for asset in [
        "swagger-ui.css",
        "swagger-ui-bundle.js",
        "swagger-ui-standalone-preset.js",
        "swagger-initializer.js",
    ] {
        assert!(html.contains(asset), "missing Swagger UI asset: {asset}");
    }

    let initializer = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api-docs/swagger-initializer.js")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(initializer.status(), StatusCode::OK);
    let initializer = read_text_body(initializer).await;
    assert!(initializer.contains(r#""url": "/openapi.json""#));
    assert!(initializer.contains(r#""validatorUrl": "none""#));

    for path in ["/openapi.json", "/api-docs/missing.js"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(
            response.headers()["content-security-policy"],
            STRICT_CONTENT_SECURITY_POLICY,
            "{path}"
        );
    }
}

async fn served_document() -> Value {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/openapi.json")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    read_json_body(response).await
}

fn operation_at<'a>(document: &'a Value, path: &str, method: &str) -> &'a Value {
    document["paths"][path]
        .get(method)
        .unwrap_or_else(|| panic!("missing operation {method} {path}"))
}

fn set<'a>(values: &'a [&'a str]) -> BTreeSet<&'a str> {
    values.iter().copied().collect()
}

fn content_types(response: &Value) -> BTreeSet<&str> {
    response["content"]
        .as_object()
        .expect("response content should be an object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn assert_response_headers(response: &Value, bodyless_success: bool) {
    let headers = response["headers"]
        .as_object()
        .expect("response headers should be an object");
    for name in COMMON_RESPONSE_HEADERS {
        assert!(headers.contains_key(name), "missing response header {name}");
    }
    assert_eq!(headers["Cache-Control"]["schema"]["const"], "no-store");
    assert_eq!(
        headers["X-Content-Type-Options"]["schema"]["const"],
        "nosniff"
    );
    assert_eq!(headers["X-Frame-Options"]["schema"]["const"], "DENY");
    assert_eq!(
        headers["Referrer-Policy"]["schema"]["const"],
        "strict-origin-when-cross-origin"
    );
    let request_id = &headers["X-Request-ID"];
    assert_eq!(
        request_id["description"],
        "Selected request correlation identifier"
    );
    assert_eq!(request_id["schema"]["type"], "string");
    assert_eq!(request_id["schema"]["minLength"], 1);
    assert_eq!(request_id["schema"]["maxLength"], 128);
    assert_eq!(
        request_id["schema"]["pattern"],
        "^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"
    );
    assert_eq!(headers.contains_key("Vary"), !bodyless_success);
    if !bodyless_success {
        assert_eq!(headers["Vary"]["description"], "Includes Accept");
        assert_eq!(headers["Vary"]["schema"]["type"], "string");
    }
}

fn assert_parameter_schema(name: &str, schema: &Value) {
    match name {
        "owner" => {
            assert_eq!(schema["minLength"], 1);
            assert_eq!(schema["maxLength"], 39);
            assert_eq!(
                schema["pattern"],
                "^[A-Za-z0-9](?:[A-Za-z0-9_-]{0,37}[A-Za-z0-9])?$"
            );
        }
        "repo" => {
            assert_eq!(schema["minLength"], 1);
            assert_eq!(schema["maxLength"], 100);
            assert_eq!(schema["pattern"], "^(?=.*[A-Za-z0-9_-])[A-Za-z0-9._-]+$");
        }
        "limit" => {
            assert_eq!(schema["minimum"], 1);
            assert_eq!(schema["maximum"], 100);
            assert_eq!(schema["default"], 20);
        }
        "cursor" => {
            assert_eq!(schema["minLength"], 1);
            assert_eq!(schema["maxLength"], 2048);
            assert_eq!(schema["pattern"], "^[!-~]+$");
        }
        "category" => {
            assert_eq!(
                schema["enum"],
                json!([
                    "electronics",
                    "tools",
                    "accessories",
                    "robotics",
                    "power",
                    "components"
                ])
            );
        }
        _ => panic!("unexpected parameter {name}"),
    }
}

fn assert_exact_problem_schema(document: &Value, path: &str, status: &str, response: &Value) {
    let expected_code = match (path, status) {
        (_, "400") => "invalid_request",
        (_, "401") => "unauthorized",
        ("/v1/profile", "404") => "profile_not_found",
        (path, "404") if path.starts_with("/v1/github/") => "github_not_found",
        (_, "406") => "not_acceptable",
        (_, "409") => "profile_exists",
        (_, "413") => "payload_too_large",
        (_, "415") => "unsupported_media_type",
        (_, "422") => "validation_failed",
        (_, "429") => "github_rate_limit",
        (_, "500") => "internal_error",
        (_, "502") => "github_upstream",
        (_, "503") => "dependency_unavailable",
        (_, "504") => "github_timeout",
        _ => panic!("unexpected error status {status} for {path}"),
    };
    for media in ["application/problem+json", "application/cbor"] {
        let schema = resolve_schema(document, &response["content"][media]["schema"]);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            property(document, schema, "status")["const"],
            status.parse::<u16>().expect("numeric status")
        );
        assert_eq!(property(document, schema, "code")["const"], expected_code);
        assert_eq!(property(document, schema, "type")["const"], "about:blank");
        assert!(
            property(document, schema, "title")["const"]
                .as_str()
                .is_some()
        );
        assert!(
            property(document, schema, "detail")["const"]
                .as_str()
                .is_some()
        );
        let errors = property(document, schema, "errors");
        assert_eq!(errors["minItems"], 1);
        assert_eq!(errors["maxItems"], 32);
    }
}

fn request_schema<'a>(document: &'a Value, path: &str, method: &str, media: &str) -> &'a Value {
    resolve_schema(
        document,
        &operation_at(document, path, method)["requestBody"]["content"][media]["schema"],
    )
}

fn response_schema<'a>(
    document: &'a Value,
    path: &str,
    method: &str,
    status: &str,
    media: &str,
) -> &'a Value {
    resolve_schema(
        document,
        &operation_at(document, path, method)["responses"][status]["content"][media]["schema"],
    )
}

fn resolve_schema<'a>(document: &'a Value, mut schema: &'a Value) -> &'a Value {
    let mut visited = BTreeSet::new();
    while let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        assert!(
            visited.insert(reference),
            "cyclic schema reference {reference}"
        );
        let pointer = reference
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("remote reference {reference}"));
        schema = document
            .pointer(pointer)
            .unwrap_or_else(|| panic!("missing reference {reference}"));
    }
    schema
}

fn property<'a>(document: &'a Value, schema: &'a Value, name: &str) -> &'a Value {
    let schema = resolve_schema(document, schema);
    let property = schema["properties"]
        .get(name)
        .unwrap_or_else(|| panic!("missing schema property {name}"));
    resolve_schema(document, property)
}

fn assert_closed_object(document: &Value, schema: &Value, properties: &[&str], required: &[&str]) {
    let schema = resolve_schema(document, schema);
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]
            .as_object()
            .expect("properties should be an object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        properties.iter().copied().collect()
    );
    assert_eq!(
        schema
            .get("required")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().expect("required member"))
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default(),
        required.iter().copied().collect()
    );
}

fn assert_bounded_name(schema: &Value) {
    assert_eq!(schema["type"], "string");
    assert_eq!(schema["minLength"], 1);
    assert_eq!(schema["maxLength"], 100);
    let pattern = schema["pattern"].as_str().expect("bounded-name pattern");
    for class in ["\\u0000-\\u001F", "\\u007F-\\u009F", "\\u00A0", "\\u3000"] {
        assert!(
            pattern.contains(class),
            "bounded-name pattern omits {class}"
        );
    }
}

fn assert_safe_integer(schema: &Value) {
    assert_eq!(schema["type"], "integer");
    assert_eq!(schema["minimum"], 0);
    assert_eq!(schema["maximum"], 9_007_199_254_740_991_u64);
}

fn assert_timestamp(schema: &Value) {
    assert_eq!(schema["type"], "string");
    assert_eq!(schema["format"], "date-time");
    let pattern = schema["pattern"].as_str().expect("timestamp pattern");
    assert!(pattern.contains("[0-5][0-9]\\.[0-9]{3}Z$"));
}

fn assert_input_contact_schemas(document: &Value, schema: &Value) {
    let email = property(document, schema, "contactEmail");
    let phone = property(document, schema, "phoneNumber");
    assert_eq!(email["type"], "string");
    assert!(email.get("maxLength").is_none());
    assert!(email["pattern"].as_str().is_some_and(|pattern| {
        pattern.starts_with("^[\\t-\\r ]*")
            && pattern.contains('`')
            && pattern.ends_with("[\\t-\\r ]*$")
    }));
    assert_eq!(phone["type"], "string");
    assert!(phone.get("maxLength").is_none());
    assert_eq!(
        phone["pattern"],
        "^[\\t-\\r ]*\\+[1-9][0-9]{6,14}[\\t-\\r ]*$"
    );
}

fn assert_github_schemas(document: &Value) {
    let owner = response_schema(
        document,
        "/v1/github/owners/{owner}",
        "get",
        "200",
        "application/json",
    );
    let owner_fields = [
        "id",
        "login",
        "type",
        "name",
        "avatarUrl",
        "htmlUrl",
        "company",
        "blog",
        "location",
        "bio",
        "publicRepos",
        "followers",
        "following",
        "createdAt",
        "updatedAt",
    ];
    assert_closed_object(document, owner, &owner_fields, &owner_fields);
    for name in ["name", "company", "blog", "location", "bio"] {
        assert!(allows_null(property(document, owner, name)));
    }
    for name in ["avatarUrl", "htmlUrl"] {
        let url = property(document, owner, name);
        assert_eq!(url["format"], "uri");
        assert!(
            url["pattern"]
                .as_str()
                .is_some_and(|pattern| pattern.contains("[Hh][Tt][Tt][Pp]"))
        );
    }

    let repositories = response_schema(
        document,
        "/v1/github/owners/{owner}/repos",
        "get",
        "200",
        "application/json",
    );
    assert_closed_object(
        document,
        repositories,
        &["repos", "count"],
        &["repos", "count"],
    );
    assert_eq!(property(document, repositories, "repos")["maxItems"], 100);
    let summary = resolve_schema(
        document,
        &property(document, repositories, "repos")["items"],
    );
    let summary_fields = ["id", "name", "fullName", "description", "htmlUrl", "fork"];
    assert_closed_object(document, summary, &summary_fields, &summary_fields);
    assert!(allows_null(property(document, summary, "description")));
    assert_repository_strings(document, summary);

    let repository = response_schema(
        document,
        "/v1/github/repos/{owner}/{repo}",
        "get",
        "200",
        "application/json",
    );
    let repository_fields = [
        "id",
        "name",
        "fullName",
        "description",
        "htmlUrl",
        "fork",
        "language",
        "stargazersCount",
        "forksCount",
        "openIssuesCount",
        "archived",
        "createdAt",
        "updatedAt",
        "pushedAt",
        "defaultBranch",
        "license",
        "topics",
        "disabled",
    ];
    assert_closed_object(document, repository, &repository_fields, &repository_fields);
    assert_repository_strings(document, repository);
    for name in ["description", "language", "pushedAt", "license"] {
        assert!(
            allows_null(property(document, repository, name)),
            "{name} should be required nullable"
        );
    }
    assert_eq!(
        property(document, repository, "topics")["uniqueItems"],
        true
    );

    let activities = response_schema(
        document,
        "/v1/github/repos/{owner}/{repo}/activity",
        "get",
        "200",
        "application/json",
    );
    assert_closed_object(
        document,
        activities,
        &["activities", "count"],
        &["activities", "count"],
    );
    assert_eq!(
        property(document, activities, "activities")["maxItems"],
        100
    );
    let activity = resolve_schema(
        document,
        &property(document, activities, "activities")["items"],
    );
    let activity_fields = [
        "id",
        "actor",
        "actorAvatarUrl",
        "ref",
        "timestamp",
        "activityType",
    ];
    assert_closed_object(document, activity, &activity_fields, &activity_fields);
    assert!(allows_null(property(document, activity, "actor")));
    assert!(allows_null(property(document, activity, "actorAvatarUrl")));
    assert!(
        activity["allOf"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty())
    );

    let tags = response_schema(
        document,
        "/v1/github/repos/{owner}/{repo}/tags",
        "get",
        "200",
        "application/json",
    );
    assert_eq!(property(document, tags, "tags")["maxItems"], 100);
    let tag = resolve_schema(document, &property(document, tags, "tags")["items"]);
    assert_eq!(
        property(document, property(document, tag, "commit"), "sha")["pattern"],
        "^(?:[0-9a-f]{40}|[0-9a-f]{64})$"
    );
}

fn assert_repository_strings(document: &Value, schema: &Value) {
    for name in ["name", "fullName"] {
        assert_eq!(property(document, schema, name)["minLength"], 1);
    }
    let description = property(document, schema, "description");
    assert!(allows_null(description));
    assert_eq!(description["minLength"], 1);
    let html_url = property(document, schema, "htmlUrl");
    assert_eq!(html_url["format"], "uri");
    assert!(
        html_url["pattern"]
            .as_str()
            .is_some_and(|pattern| pattern.starts_with("^[Hh][Tt][Tt][Pp]"))
    );
}

fn allows_null(schema: &Value) -> bool {
    schema["type"]
        .as_array()
        .is_some_and(|types| types.iter().any(|kind| kind == "null"))
        || schema["oneOf"]
            .as_array()
            .is_some_and(|schemas| schemas.iter().any(allows_null))
        || schema["type"] == "null"
}

fn assert_no_nullable_keyword(value: &Value) {
    match value {
        Value::Object(object) => {
            assert!(
                !object.contains_key("nullable"),
                "OpenAPI 3.0 nullable keyword is forbidden"
            );
            object.values().for_each(assert_no_nullable_keyword);
        }
        Value::Array(values) => values.iter().for_each(assert_no_nullable_keyword),
        _ => {}
    }
}

fn collect_local_references<'a>(
    value: &'a Value,
    document: &'a Value,
    references: &mut BTreeSet<&'a str>,
) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let pointer = reference
                    .strip_prefix('#')
                    .unwrap_or_else(|| panic!("remote OpenAPI reference {reference}"));
                assert!(
                    document.pointer(pointer).is_some(),
                    "unresolved OpenAPI reference {reference}"
                );
                references.insert(reference);
            }
            object
                .values()
                .for_each(|child| collect_local_references(child, document, references));
        }
        Value::Array(values) => values
            .iter()
            .for_each(|child| collect_local_references(child, document, references)),
        _ => {}
    }
}
