mod common;

use std::convert::Infallible;

use axum::{
    body::{Body, Bytes},
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{
    MockAuthVerifier, MockGitHubService, MockProfileService, build_app,
    pagination::cursor::{Cursor, CursorDirection, CursorScope},
    problem::{ProblemCode, ProblemDetails},
    services::github::{
        GitHubPagination, GitHubRateLimit, GitHubServiceError, GitHubUpstreamErrorKind,
        ProviderPage,
    },
};
use futures_util::stream;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, state_with};

async fn request(mock: MockGitHubService, target: &str) -> axum::response::Response {
    build_app(state_with(
        MockAuthVerifier::test_user(),
        mock,
        MockProfileService::default(),
    ))
    .oneshot(Request::builder().uri(target).body(Body::empty()).unwrap())
    .await
    .unwrap()
}

async fn assert_problem(
    mock: MockGitHubService,
    target: &str,
    status: StatusCode,
    code: ProblemCode,
) {
    let response = request(mock, target).await;
    assert_eq!(response.status(), status, "{target}");
    let body: ProblemDetails = read_json_body(response).await;
    assert_eq!(body.status, status.as_u16());
    assert_eq!(body.code, code);
    assert_eq!(body.detail, code.detail());
}

fn relation(link: &str, name: &str) -> Option<String> {
    link.split(", ").find_map(|member| {
        member
            .ends_with(&format!("; rel=\"{name}\""))
            .then(|| {
                member
                    .strip_prefix('<')?
                    .split_once('>')
                    .map(|(target, _)| target.to_owned())
            })
            .flatten()
    })
}

#[tokio::test]
async fn all_six_github_operations_return_the_exact_public_projection() {
    let cases = [
        ("/v1/github/owners/octocat", "getGitHubOwner"),
        (
            "/v1/github/owners/octocat/repos",
            "listGitHubOwnerRepositories",
        ),
        (
            "/v1/github/repos/octocat/Hello-World",
            "getGitHubRepository",
        ),
        (
            "/v1/github/repos/octocat/Hello-World/activity",
            "listGitHubRepositoryActivity",
        ),
        (
            "/v1/github/repos/octocat/Hello-World/languages",
            "listGitHubRepositoryLanguages",
        ),
        (
            "/v1/github/repos/octocat/Hello-World/tags",
            "listGitHubRepositoryTags",
        ),
    ];
    for (target, operation) in cases {
        let mock = MockGitHubService::demo();
        let response = request(mock.clone(), target).await;
        assert_eq!(response.status(), StatusCode::OK, "{operation}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let body: Value = read_json_body(response).await;
        assert!(body.is_object());
        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].operation, operation);

        match operation {
            "getGitHubOwner" => {
                let keys = body
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                assert_eq!(
                    keys,
                    [
                        "avatarUrl",
                        "bio",
                        "blog",
                        "company",
                        "createdAt",
                        "followers",
                        "following",
                        "htmlUrl",
                        "id",
                        "location",
                        "login",
                        "name",
                        "publicRepos",
                        "type",
                        "updatedAt"
                    ]
                );
                assert_eq!(body["id"], 583231);
                assert_eq!(body["bio"], Value::Null);
                assert!(body.get("email").is_none());
            }
            "listGitHubOwnerRepositories" => {
                assert_eq!(body["count"], 1);
                assert_eq!(
                    body["repos"][0],
                    json!({
                        "id":1296269, "name":"Hello-World", "fullName":"octocat/Hello-World",
                        "description":"Synthetic repository fixture",
                        "htmlUrl":"https://github.com/octocat/Hello-World", "fork":false
                    })
                );
            }
            "getGitHubRepository" => {
                assert_eq!(body["language"], "Rust");
                assert_eq!(body["stargazersCount"], 42);
                assert_eq!(body["license"], "MIT");
                assert_eq!(body["topics"], json!(["example"]));
                for forbidden in ["private", "visibility", "owner"] {
                    assert!(body.get(forbidden).is_none());
                }
            }
            "listGitHubRepositoryActivity" => {
                assert_eq!(body["count"], 1);
                assert_eq!(body["activities"][0]["actor"], "octocat");
                assert_eq!(body["activities"][0]["ref"], "refs/heads/main");
                assert!(body["activities"][0].get("pusher").is_none());
            }
            "listGitHubRepositoryLanguages" => {
                assert_eq!(body, json!({"languages":[{"name":"Rust","bytes":6789}]}));
            }
            "listGitHubRepositoryTags" => {
                assert_eq!(body["count"], 1);
                assert_eq!(body["tags"][0]["name"], "v1.0.0");
                assert_eq!(
                    body["tags"][0]["commit"]["sha"],
                    "0123456789abcdef0123456789abcdef01234567"
                );
            }
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn github_successes_and_failures_support_cbor_without_invoking_auth() {
    let auth = MockAuthVerifier::test_user();
    let mock = MockGitHubService::demo();
    let app = build_app(state_with(
        auth.clone(),
        mock.clone(),
        MockProfileService::default(),
    ));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/github/owners/octocat")
                .header(header::AUTHORIZATION, "Bearer caller-token")
                .header(header::COOKIE, "session=secret")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/cbor");
    let owner: Value = read_cbor_body(response).await;
    assert_eq!(owner["login"], "octocat");

    let failure = app
        .oneshot(
            Request::builder()
                .uri("/v1/github/owners/octocat?unknown=1")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(failure.status(), StatusCode::BAD_REQUEST);
    assert_eq!(failure.headers()[header::CONTENT_TYPE], "application/cbor");
    assert_eq!(
        read_cbor_body::<ProblemDetails>(failure).await.code,
        ProblemCode::InvalidRequest
    );
    assert_eq!(auth.call_count(), 0);
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn github_path_validation_covers_exact_boundaries_without_fetching() {
    for target in [
        "/v1/github/owners/a",
        "/v1/github/owners/a_b",
        "/v1/github/repos/a/_",
        "/v1/github/repos/a/-",
        "/v1/github/repos/a/a.b",
    ] {
        let mock = MockGitHubService::demo();
        assert_eq!(
            request(mock.clone(), target).await.status(),
            StatusCode::OK,
            "{target}"
        );
        assert_eq!(mock.call_count(), 1);
    }
    let owner_39 = format!("/v1/github/owners/a{}z", "x".repeat(37));
    let repo_100 = format!("/v1/github/repos/a/{}", "x".repeat(100));
    for target in [owner_39, repo_100] {
        let mock = MockGitHubService::demo();
        assert_eq!(
            request(mock.clone(), &target).await.status(),
            StatusCode::OK,
            "{target}"
        );
        assert_eq!(mock.call_count(), 1);
    }

    let invalid = [
        "/v1/github/owners/-a",
        "/v1/github/owners/a-",
        "/v1/github/owners/a.b",
        "/v1/github/owners/%C3%A9",
        "/v1/github/repos/a/.",
        "/v1/github/repos/a/..",
        "/v1/github/repos/a/...",
        "/v1/github/repos/a/%C3%A9",
    ];
    for target in invalid {
        let mock = MockGitHubService::demo();
        let response = request(mock.clone(), target).await;
        assert!(
            matches!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY | StatusCode::NOT_FOUND
            ),
            "{target}: {}",
            response.status()
        );
        assert_eq!(mock.call_count(), 0, "{target}");
    }
}

#[tokio::test]
async fn github_closed_query_and_limit_rejections_make_no_fetch() {
    for target in [
        "/v1/github/owners/octocat?limit=1",
        "/v1/github/repos/octocat/repo?x=1",
        "/v1/github/repos/octocat/repo/languages?cursor=x",
        "/v1/github/owners/octocat/repos?unknown=1",
        "/v1/github/owners/octocat/repos?limit=1&limit=2",
        "/v1/github/owners/octocat/repos?cursor=%",
        "/v1/github/owners/octocat/repos?cursor=%FF",
        "/v1/github/owners/octocat/repos?cursor=",
    ] {
        let mock = MockGitHubService::demo();
        assert_problem(
            mock.clone(),
            target,
            StatusCode::BAD_REQUEST,
            ProblemCode::InvalidRequest,
        )
        .await;
        assert_eq!(mock.call_count(), 0, "{target}");
    }
    for target in [
        "/v1/github/owners/octocat/repos?limit=0",
        "/v1/github/owners/octocat/repos?limit=101",
        "/v1/github/owners/octocat/repos?limit=-1",
        "/v1/github/owners/octocat/repos?limit=1.0",
        "/v1/github/owners/octocat/repos?limit=999999999999",
    ] {
        let mock = MockGitHubService::demo();
        assert_problem(
            mock.clone(),
            target,
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
        )
        .await;
        assert_eq!(mock.call_count(), 0, "{target}");
    }
}

#[tokio::test]
async fn github_numbered_navigation_is_translated_to_scoped_relative_links() {
    let mock = MockGitHubService::demo().with_repos_page(ProviderPage {
        items: Vec::new(),
        next: Some("3".to_owned()),
        prev: Some("1".to_owned()),
    });
    let response = request(mock.clone(), "/v1/github/owners/octocat/repos?limit=10").await;
    let link = response.headers()[header::LINK]
        .to_str()
        .unwrap()
        .to_owned();
    let next = relation(&link, "next").unwrap();
    let prev = relation(&link, "prev").unwrap();
    for target in [&next, &prev] {
        assert!(target.starts_with("/v1/github/owners/octocat/repos?limit=10&cursor="));
        assert!(!target.contains("api.github"));
    }
    let _: Value = read_json_body(response).await;
    assert_eq!(request(mock.clone(), &next).await.status(), StatusCode::OK);
    assert_eq!(request(mock.clone(), &prev).await.status(), StatusCode::OK);
    let calls = mock.calls();
    assert_eq!(calls[1].pagination, Some(GitHubPagination::Numbered(3)));
    assert_eq!(calls[2].pagination, Some(GitHubPagination::Numbered(1)));
}

#[tokio::test]
async fn github_activity_navigation_maps_direction_to_after_or_before() {
    let scope = CursorScope {
        operation: "listGitHubRepositoryActivity",
        owner: Some("octocat"),
        repository: Some("repo"),
        limit: 20,
        category: None,
    };
    for direction in [CursorDirection::Next, CursorDirection::Prev] {
        let cursor = Cursor::new(&scope, direction, "provider-token").encode();
        let mock = MockGitHubService::demo();
        assert_eq!(
            request(
                mock.clone(),
                &format!("/v1/github/repos/octocat/repo/activity?cursor={cursor}")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            mock.calls()[0].pagination,
            Some(GitHubPagination::Activity {
                direction,
                value: "provider-token".to_owned()
            })
        );
    }
}

#[tokio::test]
async fn github_rejects_provider_navigation_that_cannot_fit_a_public_cursor() {
    let mock = MockGitHubService::demo().with_activity_page(ProviderPage {
        items: Vec::new(),
        next: Some("x".repeat(2_048)),
        prev: None,
    });
    let response = request(
        mock.clone(),
        "/v1/github/repos/octocat/repo/activity?limit=100",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(response.headers().get(header::LINK).is_none());
    let problem: ProblemDetails = read_json_body(response).await;
    assert_eq!(problem.code, ProblemCode::GithubUpstream);
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn github_cursors_bind_operation_resource_limit_direction_and_value() {
    let scope = CursorScope {
        operation: "listGitHubOwnerRepositories",
        owner: Some("octocat"),
        repository: None,
        limit: 20,
        category: None,
    };
    let valid = Cursor::new(&scope, CursorDirection::Next, "2").encode();
    let mock = MockGitHubService::demo();
    assert_eq!(
        request(
            mock.clone(),
            &format!("/v1/github/owners/octocat/repos?cursor={valid}")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(mock.call_count(), 1);

    let wrong = [
        Cursor::new(
            &CursorScope {
                operation: "listGitHubRepositoryTags",
                ..scope
            },
            CursorDirection::Next,
            "2",
        )
        .encode(),
        Cursor::new(
            &CursorScope {
                owner: Some("other"),
                ..scope
            },
            CursorDirection::Next,
            "2",
        )
        .encode(),
        Cursor::new(
            &CursorScope { limit: 10, ..scope },
            CursorDirection::Next,
            "2",
        )
        .encode(),
        Cursor::new(&scope, CursorDirection::Next, "1").encode(),
        Cursor::new(&scope, CursorDirection::Next, "01").encode(),
        Cursor::new(&scope, CursorDirection::Next, "9007199254740992").encode(),
    ];
    for cursor in wrong {
        let mock = MockGitHubService::demo();
        assert_problem(
            mock.clone(),
            &format!("/v1/github/owners/octocat/repos?cursor={cursor}"),
            StatusCode::BAD_REQUEST,
            ProblemCode::InvalidRequest,
        )
        .await;
        assert_eq!(mock.call_count(), 0);
    }
    let oversized = format!(
        "/v1/github/owners/octocat/repos?cursor={}",
        "x".repeat(2049)
    );
    let mock = MockGitHubService::demo();
    assert_problem(
        mock.clone(),
        &oversized,
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;
    assert_eq!(mock.call_count(), 0);
}

#[tokio::test]
async fn github_local_negotiation_and_method_rejections_do_not_fetch_or_consume_body() {
    let mock = MockGitHubService::demo();
    let app = build_app(state_with(
        MockAuthVerifier::test_user(),
        mock.clone(),
        MockProfileService::default(),
    ));
    let unacceptable = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/github/owners/octocat")
                .header(header::ACCEPT, "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unacceptable.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(mock.call_count(), 0);

    let body = Body::from_stream(stream::once(async {
        panic!("GitHub GET polled request content");
        #[allow(unreachable_code)]
        Ok::<Bytes, Infallible>(Bytes::new())
    }));
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/github/owners/octocat")
                    .body(body)
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    let method = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/github/owners/octocat")
                .body(Body::from(vec![0; 1_000_001]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(method.headers()[header::ALLOW], "GET");
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn github_service_errors_map_to_exact_public_codes_and_quota_headers() {
    let cases = [
        (
            GitHubServiceError::NotFound,
            StatusCode::NOT_FOUND,
            ProblemCode::GithubNotFound,
        ),
        (
            GitHubServiceError::Timeout,
            StatusCode::GATEWAY_TIMEOUT,
            ProblemCode::GithubTimeout,
        ),
    ];
    for (error, status, code) in cases {
        assert_problem(
            MockGitHubService::demo().with_error(error),
            "/v1/github/owners/octocat",
            status,
            code,
        )
        .await;
    }
    assert_problem(
        MockGitHubService::demo().with_upstream_error(GitHubUpstreamErrorKind::Schema),
        "/v1/github/owners/octocat",
        StatusCode::BAD_GATEWAY,
        ProblemCode::GithubUpstream,
    )
    .await;

    let rate =
        MockGitHubService::demo().with_error(GitHubServiceError::RateLimited(GitHubRateLimit {
            retry_after: "17".to_owned(),
            rate_limit_reset: Some("9007199254740991".to_owned()),
        }));
    let response = request(rate, "/v1/github/owners/octocat").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[header::RETRY_AFTER], "17");
    assert_eq!(response.headers()["x-ratelimit-reset"], "9007199254740991");
    let problem: ProblemDetails = read_json_body(response).await;
    assert_eq!(problem.code, ProblemCode::GithubRateLimit);
}
