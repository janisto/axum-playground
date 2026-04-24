use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    http::codec::{success_response, success_response_with_headers},
    pagination::{
        cursor::{Cursor, decode_cursor},
        link::build_link_header,
    },
    problem::problem_response,
    services::github::{
        Activity, GitHubServiceError, GitHubUpstreamErrorKind, Language, Owner, Repo, RepoSummary,
        Tag,
    },
    state::AppState,
};

const ACTIVITY_CURSOR_KIND: &str = "gh-activity";
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Deserialize, ToSchema)]
pub struct OwnerPath {
    pub owner: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RepoPath {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OwnerReposResponse {
    pub repos: Vec<RepoSummary>,
    pub count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RepoActivityResponse {
    pub activities: Vec<Activity>,
    pub count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RepoLanguagesResponse {
    pub languages: Vec<Language>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RepoTagsResponse {
    pub tags: Vec<Tag>,
    pub count: usize,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/github/owners/{owner}", get(get_github_owner_handler))
        .route(
            "/github/owners/{owner}/repos",
            get(list_github_owner_repos_handler),
        )
        .route("/github/repos/{owner}/{repo}", get(get_github_repo_handler))
        .route(
            "/github/repos/{owner}/{repo}/activity",
            get(list_github_repo_activity_handler),
        )
        .route(
            "/github/repos/{owner}/{repo}/languages",
            get(get_github_repo_languages_handler),
        )
        .route(
            "/github/repos/{owner}/{repo}/tags",
            get(list_github_repo_tags_handler),
        )
}

#[utoipa::path(
    get,
    path = "/v1/github/owners/{owner}",
    tag = "GitHub",
    params(("owner" = String, Path, description = "GitHub username")),
    responses(
        (status = 200, description = "GitHub owner", content((Owner = "application/json"), (Owner = "application/cbor"))),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Resource not found"),
        (status = 429, description = "Rate limited"),
        (status = 502, description = "Upstream error")
    )
)]
pub async fn get_github_owner_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<OwnerPath>,
) -> Response {
    match state.github_service.get_owner(&path.owner).await {
        Ok(owner) => success_response(StatusCode::OK, &headers, &owner),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/owners/{owner}/repos",
    tag = "GitHub",
    params(("owner" = String, Path, description = "GitHub username")),
    responses(
        (status = 200, description = "GitHub repositories", content((OwnerReposResponse = "application/json"), (OwnerReposResponse = "application/cbor"))),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Resource not found"),
        (status = 429, description = "Rate limited"),
        (status = 502, description = "Upstream error")
    )
)]
pub async fn list_github_owner_repos_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<OwnerPath>,
) -> Response {
    match state.github_service.list_repos(&path.owner).await {
        Ok(repos) => success_response(
            StatusCode::OK,
            &headers,
            &OwnerReposResponse {
                count: repos.len(),
                repos,
            },
        ),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}",
    tag = "GitHub",
    params(
        ("owner" = String, Path, description = "GitHub username"),
        ("repo" = String, Path, description = "Repository name")
    ),
    responses(
        (status = 200, description = "GitHub repository", content((Repo = "application/json"), (Repo = "application/cbor"))),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Resource not found"),
        (status = 429, description = "Rate limited"),
        (status = 502, description = "Upstream error")
    )
)]
pub async fn get_github_repo_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<RepoPath>,
) -> Response {
    match state.github_service.get_repo(&path.owner, &path.repo).await {
        Ok(repo) => success_response(StatusCode::OK, &headers, &repo),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}/activity",
    tag = "GitHub",
    params(
        ("owner" = String, Path, description = "GitHub username"),
        ("repo" = String, Path, description = "Repository name"),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor from previous response"),
        ("limit" = Option<i64>, Query, description = "Maximum items per page", minimum = 1, maximum = 100)
    ),
    responses(
        (status = 200, description = "Repository activity", headers(("Link" = String, description = "RFC 8288 pagination links")), content((RepoActivityResponse = "application/json"), (RepoActivityResponse = "application/cbor"))),
        (status = 400, description = "Cursor validation failure"),
        (status = 422, description = "Query validation failure"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Resource not found"),
        (status = 429, description = "Rate limited"),
        (status = 502, description = "Upstream error")
    )
)]
pub async fn list_github_repo_activity_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<RepoPath>,
    Query(query): Query<ActivityQuery>,
) -> Response {
    if let Some(limit) = query.limit
        && limit > MAX_LIMIT
    {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
    }

    let cursor = match decode_cursor(query.cursor.as_deref().unwrap_or_default()) {
        Ok(cursor) => cursor,
        Err(_) => {
            return problem_response(StatusCode::BAD_REQUEST, "invalid cursor format", &headers);
        }
    };

    if !cursor.kind.is_empty() && cursor.kind != ACTIVITY_CURSOR_KIND {
        return problem_response(StatusCode::BAD_REQUEST, "cursor type mismatch", &headers);
    }

    let limit = match query.limit {
        Some(limit) if limit > 0 => limit as usize,
        _ => DEFAULT_LIMIT,
    };

    match state
        .github_service
        .list_activity(&path.owner, &path.repo, limit, &cursor.value)
        .await
    {
        Ok(page) => {
            let base_path = format!("/v1/github/repos/{}/{}/activity", path.owner, path.repo);
            let limit_string = limit.to_string();
            let query_pairs = [("limit", limit_string.as_str())];
            let next_cursor = (!page.next_cursor.is_empty())
                .then(|| Cursor::new(ACTIVITY_CURSOR_KIND, page.next_cursor).encode());
            let link_header =
                build_link_header(&base_path, &query_pairs, next_cursor.as_deref(), None);

            let extra_headers = if !link_header.is_empty() {
                vec![(
                    header::LINK,
                    HeaderValue::from_str(&link_header).expect("link header should be valid"),
                )]
            } else {
                Vec::new()
            };

            success_response_with_headers(
                StatusCode::OK,
                &headers,
                &RepoActivityResponse {
                    count: page.activities.len(),
                    activities: page.activities,
                },
                extra_headers,
            )
        }
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}/languages",
    tag = "GitHub",
    params(
        ("owner" = String, Path, description = "GitHub username"),
        ("repo" = String, Path, description = "Repository name")
    ),
    responses(
        (status = 200, description = "Repository languages", content((RepoLanguagesResponse = "application/json"), (RepoLanguagesResponse = "application/cbor"))),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Resource not found"),
        (status = 429, description = "Rate limited"),
        (status = 502, description = "Upstream error")
    )
)]
pub async fn get_github_repo_languages_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<RepoPath>,
) -> Response {
    match state
        .github_service
        .list_languages(&path.owner, &path.repo)
        .await
    {
        Ok(languages) => success_response(
            StatusCode::OK,
            &headers,
            &RepoLanguagesResponse { languages },
        ),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}/tags",
    tag = "GitHub",
    params(
        ("owner" = String, Path, description = "GitHub username"),
        ("repo" = String, Path, description = "Repository name")
    ),
    responses(
        (status = 200, description = "Repository tags", content((RepoTagsResponse = "application/json"), (RepoTagsResponse = "application/cbor"))),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Resource not found"),
        (status = 429, description = "Rate limited"),
        (status = 502, description = "Upstream error")
    )
)]
pub async fn list_github_repo_tags_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<RepoPath>,
) -> Response {
    match state
        .github_service
        .list_tags(&path.owner, &path.repo)
        .await
    {
        Ok(tags) => success_response(
            StatusCode::OK,
            &headers,
            &RepoTagsResponse {
                count: tags.len(),
                tags,
            },
        ),
        Err(error) => map_service_error(&headers, error),
    }
}

fn map_service_error(headers: &HeaderMap, error: GitHubServiceError) -> Response {
    match error {
        GitHubServiceError::NotFound => {
            problem_response(StatusCode::NOT_FOUND, "resource not found", headers)
        }
        GitHubServiceError::Forbidden => {
            problem_response(StatusCode::FORBIDDEN, "access denied", headers)
        }
        GitHubServiceError::RateLimited => problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
            headers,
        ),
        GitHubServiceError::Upstream(error) => match error.kind {
            GitHubUpstreamErrorKind::NotFound => {
                problem_response(StatusCode::NOT_FOUND, "resource not found", headers)
            }
            GitHubUpstreamErrorKind::Forbidden => {
                problem_response(StatusCode::FORBIDDEN, "access denied", headers)
            }
            GitHubUpstreamErrorKind::RateLimited => {
                let mut response = problem_response(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate limit exceeded",
                    headers,
                );
                if let Some(retry_after) = error.retry_after {
                    response.headers_mut().insert(
                        header::RETRY_AFTER,
                        HeaderValue::from_str(&retry_after)
                            .expect("retry-after header should be valid"),
                    );
                }
                if let Some(rate_limit_reset) = error.rate_limit_reset {
                    response.headers_mut().insert(
                        HeaderName::from_static("x-ratelimit-reset"),
                        HeaderValue::from_str(&rate_limit_reset)
                            .expect("rate limit reset header should be valid"),
                    );
                }
                response
            }
            GitHubUpstreamErrorKind::Upstream => {
                problem_response(StatusCode::BAD_GATEWAY, "upstream error", headers)
            }
        },
    }
}
