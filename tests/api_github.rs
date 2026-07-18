mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{
    GitHubActivity, GitHubActivityPage, GitHubListPage, GitHubRepoSummary, GitHubService,
    GitHubServiceError, GitHubTag, GitHubTagCommit, GitHubUpstreamError, GitHubUpstreamErrorKind,
    MockGitHubService, build_app, problem::ProblemDetails,
};
use serde::Deserialize;
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, test_state, test_state_with_github_service};

#[derive(Debug, Deserialize)]
struct Owner {
    login: String,
    company: String,
}

#[derive(Debug, Deserialize)]
struct RepoSummary {
    name: String,
}

#[derive(Debug, Deserialize)]
struct OwnerReposResponse {
    repos: Vec<RepoSummary>,
    count: usize,
}

#[derive(Debug, Deserialize)]
struct Repo {
    name: String,
    #[serde(rename = "defaultBranch")]
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct RepoActivityResponse {
    activities: Vec<Activity>,
    count: usize,
}

#[derive(Debug, Deserialize)]
struct Activity {
    actor: Option<String>,
    #[serde(rename = "activityType")]
    activity_type: String,
}

#[derive(Debug, Deserialize)]
struct RepoLanguagesResponse {
    languages: Vec<Language>,
}

#[derive(Debug, Deserialize)]
struct Language {
    name: String,
    bytes: i64,
}

#[derive(Debug, Deserialize)]
struct RepoTagsResponse {
    tags: Vec<Tag>,
    count: usize,
}

#[derive(Debug, Deserialize)]
struct Tag {
    name: String,
}

#[tokio::test]
async fn github_routes_return_demo_data_and_support_cbor() {
    let owner_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/owners/octocat")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(owner_response.status(), StatusCode::OK);
    let owner: Owner = read_cbor_body(owner_response).await;
    assert_eq!(owner.login, "octocat");
    assert_eq!(owner.company, "@github");

    let repos_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/owners/octocat/repos")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let repos: OwnerReposResponse = read_json_body(repos_response).await;
    assert_eq!(repos.count, 1);
    assert_eq!(
        repos.repos.first().map(|repo| repo.name.as_str()),
        Some("git-consortium")
    );

    let repo_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let repo: Repo = read_json_body(repo_response).await;
    assert_eq!(repo.name, "git-consortium");
    assert_eq!(repo.default_branch, "master");
}

#[tokio::test]
async fn github_activity_languages_and_tags_routes_work() {
    let activity_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/activity")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let activity: RepoActivityResponse = read_json_body(activity_response).await;
    assert_eq!(activity.count, 1);
    assert_eq!(
        activity
            .activities
            .first()
            .and_then(|event| event.actor.as_deref()),
        Some("octocat")
    );
    assert_eq!(
        activity
            .activities
            .first()
            .map(|event| event.activity_type.as_str()),
        Some("push")
    );

    let languages_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/languages")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let languages: RepoLanguagesResponse = read_json_body(languages_response).await;
    assert_eq!(
        languages
            .languages
            .first()
            .map(|language| language.name.as_str()),
        Some("Ruby")
    );
    assert_eq!(
        languages.languages.first().map(|language| language.bytes),
        Some(6789)
    );

    let tags_response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/tags")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let tags: RepoTagsResponse = read_json_body(tags_response).await;
    assert_eq!(tags.count, 1);
    assert_eq!(tags.tags.first().map(|tag| tag.name.as_str()), Some("v1.0"));
}

#[tokio::test]
async fn github_repositories_and_tags_expose_upstream_next_pages_as_opaque_links() {
    let service = GitHubService::mock(
        MockGitHubService::demo()
            .with_repos_page(GitHubListPage {
                items: vec![GitHubRepoSummary {
                    name: "git-consortium".to_owned(),
                    full_name: "octocat/git-consortium".to_owned(),
                    description: "demo".to_owned(),
                    html_url: "https://github.com/octocat/git-consortium".to_owned(),
                    language: "Rust".to_owned(),
                    stars: 1,
                    forks: 2,
                    open_issues: 3,
                    created_at: "2011-01-25T18:44:36Z".to_owned(),
                    updated_at: "2024-06-01T00:00:00Z".to_owned(),
                }],
                next_cursor: "2".to_owned(),
            })
            .with_tags_page(GitHubListPage {
                items: vec![GitHubTag {
                    name: "v1.0.0".to_owned(),
                    commit: GitHubTagCommit {
                        sha: "abc123".to_owned(),
                    },
                }],
                next_cursor: "3".to_owned(),
            }),
    );
    let state = test_state_with_github_service(service);

    let repos_response = build_app(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/owners/octocat/repos?limit=50")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let repos_cursor =
        axum_playground::pagination::cursor::Cursor::new("gh-owner-repos", "2").encode();
    assert_eq!(
        repos_response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok()),
        Some(
            format!(
                "</v1/github/owners/octocat/repos?limit=50&cursor={repos_cursor}>; rel=\"next\""
            )
            .as_str()
        )
    );
    let repos: OwnerReposResponse = read_json_body(repos_response).await;
    assert_eq!(repos.count, 1);

    let tags_response = build_app(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/tags?limit=25")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let tags_cursor = axum_playground::pagination::cursor::Cursor::new("gh-tags", "3").encode();
    assert_eq!(
        tags_response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok()),
        Some(
            format!(
                "</v1/github/repos/octocat/git-consortium/tags?limit=25&cursor={tags_cursor}>; rel=\"next\""
            )
            .as_str()
        )
    );
}

#[tokio::test]
async fn github_repository_and_tag_pages_expose_previous_links() {
    let service = GitHubService::mock(
        MockGitHubService::demo()
            .with_repos_page(GitHubListPage {
                items: Vec::new(),
                next_cursor: "4".to_owned(),
            })
            .with_tags_page(GitHubListPage {
                items: Vec::new(),
                next_cursor: String::new(),
            }),
    );
    let state = test_state_with_github_service(service);

    let current_repos_cursor =
        axum_playground::pagination::cursor::Cursor::new("gh-owner-repos", "3").encode();
    let repos_response = build_app(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/github/owners/octocat/repos?limit=50&cursor={current_repos_cursor}"
                ))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let next_repos_cursor =
        axum_playground::pagination::cursor::Cursor::new("gh-owner-repos", "4").encode();
    let previous_repos_cursor =
        axum_playground::pagination::cursor::Cursor::new("gh-owner-repos", "2").encode();
    assert_eq!(
        repos_response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok()),
        Some(
            format!(
                "</v1/github/owners/octocat/repos?limit=50&cursor={next_repos_cursor}>; rel=\"next\", </v1/github/owners/octocat/repos?limit=50&cursor={previous_repos_cursor}>; rel=\"prev\""
            )
            .as_str()
        )
    );

    let current_tags_cursor =
        axum_playground::pagination::cursor::Cursor::new("gh-tags", "2").encode();
    let tags_response = build_app(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/github/repos/octocat/git-consortium/tags?limit=25&cursor={current_tags_cursor}"
                ))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let first_tags_cursor =
        axum_playground::pagination::cursor::Cursor::new("gh-tags", "").encode();
    assert_eq!(
        tags_response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok()),
        Some(
            format!(
                "</v1/github/repos/octocat/git-consortium/tags?limit=25&cursor={first_tags_cursor}>; rel=\"prev\""
            )
            .as_str()
        )
    );

    let first_tags_page = build_app(state)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/github/repos/octocat/git-consortium/tags?limit=25&cursor={first_tags_cursor}"
                ))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(first_tags_page.status(), StatusCode::OK);
    assert!(first_tags_page.headers().get(header::LINK).is_none());
}

#[tokio::test]
async fn github_repository_and_tag_pagination_is_validated_before_service_calls() {
    let cursor = |kind: &str, value: &str| {
        axum_playground::pagination::cursor::Cursor::new(kind, value).encode()
    };
    for (uri, expected_status) in [
        (
            "/v1/github/owners/octocat/repos?limit=0".to_owned(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/v1/github/repos/octocat/git-consortium/tags?limit=101".to_owned(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/v1/github/owners/octocat/repos?cursor=not-base64!".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!(
                "/v1/github/owners/octocat/repos?cursor={}",
                cursor("gh-tags", "2")
            ),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!(
                "/v1/github/repos/octocat/git-consortium/tags?cursor={}",
                cursor("gh-tags", "first")
            ),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!(
                "/v1/github/owners/octocat/repos?cursor={}",
                cursor("gh-owner-repos", "1")
            ),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let service = GitHubService::mock(
            MockGitHubService::demo().with_error(GitHubServiceError::RateLimited),
        );
        let response = build_app(test_state_with_github_service(service))
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), expected_status);
    }
}

#[tokio::test]
async fn github_activity_uses_link_header_and_validates_cursor() {
    let service = GitHubService::mock(MockGitHubService::demo().with_activity_page(
        GitHubActivityPage {
            activities: vec![
                GitHubActivity {
                    id: 1,
                    actor: Some("octocat".to_owned()),
                    git_ref: "refs/heads/master".to_owned(),
                    timestamp: "2024-01-15T10:30:00Z".to_owned(),
                    activity_type: "push".to_owned(),
                    actor_avatar_url: Some(
                        "https://avatars.githubusercontent.com/u/583231".to_owned(),
                    ),
                },
                GitHubActivity {
                    id: 2,
                    actor: None,
                    git_ref: "refs/heads/deleted".to_owned(),
                    timestamp: "2024-01-15T11:30:00Z".to_owned(),
                    activity_type: "branch_deletion".to_owned(),
                    actor_avatar_url: None,
                },
            ],
            next_cursor: "next-page-cursor".to_owned(),
        },
    ));
    let response = build_app(test_state_with_github_service(service))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/activity?limit=10")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let expected_cursor =
        axum_playground::pagination::cursor::Cursor::new("gh-activity", "next-page-cursor")
            .encode();
    assert_eq!(
        response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok()),
        Some(
            format!(
                "</v1/github/repos/octocat/git-consortium/activity?limit=10&cursor={expected_cursor}>; rel=\"next\""
            )
            .as_str()
        )
    );
    let activity: RepoActivityResponse = read_json_body(response).await;
    assert_eq!(activity.count, 2);
    assert_eq!(activity.activities[0].actor.as_deref(), Some("octocat"));
    assert_eq!(activity.activities[1].actor, None);

    let invalid_cursor = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/activity?cursor=not-valid-base64!")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(invalid_cursor.status(), StatusCode::BAD_REQUEST);

    let wrong_type_cursor =
        axum_playground::pagination::cursor::Cursor::new("wrong-type", "abc").encode();
    let wrong_type = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/github/repos/octocat/git-consortium/activity?cursor={wrong_type_cursor}"
                ))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(wrong_type.status(), StatusCode::BAD_REQUEST);

    let invalid_limit = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/activity?limit=101")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(invalid_limit.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let zero_limit = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/activity?limit=0")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(zero_limit.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let negative_limit = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/activity?limit=-10")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(negative_limit.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let maximum_limit = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/repos/octocat/git-consortium/activity?limit=100")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(maximum_limit.status(), StatusCode::OK);

    let untyped_cursor =
        axum_playground::pagination::cursor::Cursor::new("", "upstream-cursor").encode();
    let untyped = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/github/repos/octocat/git-consortium/activity?cursor={untyped_cursor}"
                ))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    assert_eq!(untyped.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn github_rejects_invalid_paths_and_query_syntax_before_service_calls() {
    let rejecting_service = || {
        GitHubService::mock(MockGitHubService::demo().with_error(GitHubServiceError::RateLimited))
    };

    for uri in [
        "/v1/github/owners/-invalid",
        "/v1/github/repos/octocat/invalid%20repo",
        "/v1/github/repos/octocat/git-consortium/activity?limit=not-a-number",
    ] {
        let response = build_app(test_state_with_github_service(rejecting_service()))
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/problem+json")
        );
        let problem: ProblemDetails = read_json_body(response).await;
        assert_eq!(problem.status, StatusCode::BAD_REQUEST.as_u16());
    }
}

#[tokio::test]
async fn github_path_validation_accepts_internal_hyphens() {
    let service =
        GitHubService::mock(MockGitHubService::demo().with_error(GitHubServiceError::NotFound));
    let response = build_app(test_state_with_github_service(service))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/github/owners/octo-cat")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn github_error_mapping_covers_not_found_forbidden_rate_limit_and_upstream() {
    let not_found = build_app(test_state_with_github_service(GitHubService::mock(
        MockGitHubService::demo().with_error(GitHubServiceError::NotFound),
    )))
    .oneshot(
        Request::builder()
            .method(Method::GET)
            .uri("/v1/github/owners/octocat")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await
    .expect("request should succeed");
    assert_eq!(not_found.status(), StatusCode::NOT_FOUND);

    let forbidden = build_app(test_state_with_github_service(GitHubService::mock(
        MockGitHubService::demo().with_error(GitHubServiceError::Forbidden),
    )))
    .oneshot(
        Request::builder()
            .method(Method::GET)
            .uri("/v1/github/owners/octocat")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await
    .expect("request should succeed");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let rate_limited = build_app(test_state_with_github_service(GitHubService::mock(
        MockGitHubService::demo().with_error(GitHubServiceError::Upstream(
            GitHubUpstreamError::new(
                GitHubUpstreamErrorKind::RateLimited,
                403,
                Some("60".to_owned()),
                Some("1700000000".to_owned()),
            ),
        )),
    )))
    .oneshot(
        Request::builder()
            .method(Method::GET)
            .uri("/v1/github/owners/octocat")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await
    .expect("request should succeed");
    assert_eq!(rate_limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        rate_limited
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("60")
    );
    assert_eq!(
        rate_limited
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|value| value.to_str().ok()),
        Some("1700000000")
    );

    let upstream = build_app(test_state_with_github_service(GitHubService::mock(
        MockGitHubService::demo().with_error(GitHubServiceError::Upstream(
            GitHubUpstreamError::new(GitHubUpstreamErrorKind::Upstream, 500, None, None),
        )),
    )))
    .oneshot(
        Request::builder()
            .method(Method::GET)
            .uri("/v1/github/owners/octocat")
            .body(Body::empty())
            .expect("request should build"),
    )
    .await
    .expect("request should succeed");
    assert_eq!(upstream.status(), StatusCode::BAD_GATEWAY);

    let problem: ProblemDetails = read_json_body(upstream).await;
    assert_eq!(problem.status, StatusCode::BAD_GATEWAY.as_u16());
}

#[tokio::test]
async fn openapi_includes_github_paths() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/openapi")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    let document: serde_json::Value = read_json_body(response).await;
    assert!(document["paths"].get("/v1/github/owners/{owner}").is_some());
    assert!(
        document["paths"]
            .get("/v1/github/repos/{owner}/{repo}/tags")
            .is_some()
    );
    for path in [
        "/v1/github/owners/{owner}/repos",
        "/v1/github/repos/{owner}/{repo}/tags",
    ] {
        let operation = &document["paths"][path]["get"];
        let parameter_names = operation["parameters"]
            .as_array()
            .expect("parameters should be an array")
            .iter()
            .filter_map(|parameter| parameter["name"].as_str())
            .collect::<Vec<_>>();
        assert!(parameter_names.contains(&"cursor"));
        assert!(parameter_names.contains(&"limit"));
        assert_eq!(
            operation["responses"]["200"]["headers"]["Link"]["schema"]["type"],
            "string"
        );
        assert_eq!(
            operation["responses"]["422"]["$ref"],
            "#/components/responses/ProblemResponse"
        );
    }
    assert_eq!(
        document["components"]["schemas"]["Activity"]["properties"]["actor"]["type"],
        serde_json::json!(["string", "null"])
    );
    assert_eq!(
        document["components"]["schemas"]["Activity"]["properties"]["actorAvatarUrl"]["type"],
        serde_json::json!(["string", "null"])
    );
    assert!(
        document["components"]["responses"]["ProblemResponse"]["content"]
            .get("application/problem+json")
            .is_some()
    );
    assert!(
        document["components"]["responses"]["ProblemResponse"]["content"]
            .get("application/cbor")
            .is_some()
    );
}
