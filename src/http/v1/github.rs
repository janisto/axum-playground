use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::Response,
    routing::{MethodFilter, on},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    http::{
        codec::{ResponseFormat, success_response, success_response_with_headers},
        extract::{NoQuery, ProblemPath, StrictQuery},
    },
    pagination::{
        cursor::{Cursor, CursorDirection, CursorScope, decode_cursor, validate_cursor_text},
        link::build_link_header,
        resolve_limit,
    },
    problem::{ProblemCode, ProblemResponse, problem_response},
    services::github::{
        Activity, GitHubPagination, GitHubServiceError, Language, Owner, ProviderPage, Repository,
        RepositorySummary, Tag, valid_github_owner, valid_github_repository,
    },
    state::AppState,
    validation::SAFE_INTEGER_MAX,
};

const DEFAULT_LIMIT: u16 = 20;
const MAX_LIMIT: u16 = 100;

#[derive(Debug, Deserialize)]
pub struct OwnerPath {
    pub owner: String,
}

#[derive(Debug, Deserialize)]
pub struct RepositoryPath {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GitHubRepositoryPage {
    pub repos: Vec<RepositorySummary>,
    #[schema(minimum = 0, maximum = 100)]
    pub count: u64,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GitHubActivityPage {
    pub activities: Vec<Activity>,
    #[schema(minimum = 0, maximum = 100)]
    pub count: u64,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GitHubLanguages {
    pub languages: Vec<Language>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GitHubTagPage {
    pub tags: Vec<Tag>,
    #[schema(minimum = 0, maximum = 100)]
    pub count: u64,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/github/owners/{owner}",
            on(MethodFilter::GET, get_github_owner_handler),
        )
        .route(
            "/github/owners/{owner}/repos",
            on(MethodFilter::GET, list_github_owner_repos_handler),
        )
        .route(
            "/github/repos/{owner}/{repo}",
            on(MethodFilter::GET, get_github_repo_handler),
        )
        .route(
            "/github/repos/{owner}/{repo}/activity",
            on(MethodFilter::GET, list_github_repo_activity_handler),
        )
        .route(
            "/github/repos/{owner}/{repo}/languages",
            on(MethodFilter::GET, get_github_repo_languages_handler),
        )
        .route(
            "/github/repos/{owner}/{repo}/tags",
            on(MethodFilter::GET, list_github_repo_tags_handler),
        )
}

#[utoipa::path(
    get,
    path = "/v1/github/owners/{owner}",
    operation_id = "getGitHubOwner",
    tag = "GitHub",
    security(()),
    params(("owner" = String, Path, min_length = 1, max_length = 39, pattern = r"^[A-Za-z0-9](?:[A-Za-z0-9_-]{0,37}[A-Za-z0-9])?$")),
    responses(
        (status = 200, description = "Public GitHub owner", content((Owner = "application/json"), (Owner = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 429, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 502, response = ProblemResponse),
        (status = 504, response = ProblemResponse)
    )
)]
pub async fn get_github_owner_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    _query: NoQuery,
    ProblemPath(path): ProblemPath<OwnerPath>,
    headers: HeaderMap,
) -> Response {
    if !valid_github_owner(&path.owner) {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }
    match state.github_service.get_owner(&path.owner).await {
        Ok(owner) => success_response(StatusCode::OK, format, &owner),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/owners/{owner}/repos",
    operation_id = "listGitHubOwnerRepositories",
    tag = "GitHub",
    security(()),
    params(
        ("owner" = String, Path, min_length = 1, max_length = 39, pattern = r"^[A-Za-z0-9](?:[A-Za-z0-9_-]{0,37}[A-Za-z0-9])?$"),
        ("limit" = Option<u16>, Query, minimum = 1, maximum = 100, description = "Default 20; closed query"),
        ("cursor" = Option<String>, Query, max_length = 2048, description = "Opaque scoped cursor; closed query")
    ),
    responses(
        (status = 200, description = "Public owner repositories", headers(("Link" = String, description = "Optional RFC 8288 navigation")), content((GitHubRepositoryPage = "application/json"), (GitHubRepositoryPage = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 429, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 502, response = ProblemResponse),
        (status = 504, response = ProblemResponse)
    )
)]
pub async fn list_github_owner_repos_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    ProblemPath(path): ProblemPath<OwnerPath>,
    headers: HeaderMap,
    query: StrictQuery,
) -> Response {
    if !valid_github_owner(&path.owner) {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }
    let (limit, cursor) = match page_query(query, "listGitHubOwnerRepositories", &path.owner, None)
    {
        Ok(value) => value,
        Err(code) => return problem_response(code, &headers),
    };
    let Ok(pagination) = numbered_pagination(cursor.as_ref()) else {
        return problem_response(ProblemCode::InvalidRequest, &headers);
    };
    match state
        .github_service
        .list_repositories(&path.owner, limit, pagination)
        .await
    {
        Ok(page) => repository_page_response(format, &path.owner, limit, page)
            .unwrap_or_else(|()| problem_response(ProblemCode::GithubUpstream, &headers)),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}",
    operation_id = "getGitHubRepository",
    tag = "GitHub",
    security(()),
    params(
        ("owner" = String, Path, min_length = 1, max_length = 39),
        ("repo" = String, Path, min_length = 1, max_length = 100, pattern = r"^[A-Za-z0-9._-]+$")
    ),
    responses(
        (status = 200, description = "Public GitHub repository", content((Repository = "application/json"), (Repository = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 429, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 502, response = ProblemResponse),
        (status = 504, response = ProblemResponse)
    )
)]
pub async fn get_github_repo_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    _query: NoQuery,
    ProblemPath(path): ProblemPath<RepositoryPath>,
    headers: HeaderMap,
) -> Response {
    if !valid_repository_path(&path) {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }
    match state
        .github_service
        .get_repository(&path.owner, &path.repo)
        .await
    {
        Ok(repository) => success_response(StatusCode::OK, format, &repository),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}/activity",
    operation_id = "listGitHubRepositoryActivity",
    tag = "GitHub",
    security(()),
    params(
        ("owner" = String, Path, min_length = 1, max_length = 39),
        ("repo" = String, Path, min_length = 1, max_length = 100),
        ("limit" = Option<u16>, Query, minimum = 1, maximum = 100, description = "Default 20; closed query"),
        ("cursor" = Option<String>, Query, max_length = 2048, description = "Opaque scoped cursor; closed query")
    ),
    responses(
        (status = 200, description = "Repository activity", headers(("Link" = String, description = "Optional RFC 8288 navigation")), content((GitHubActivityPage = "application/json"), (GitHubActivityPage = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 429, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 502, response = ProblemResponse),
        (status = 504, response = ProblemResponse)
    )
)]
pub async fn list_github_repo_activity_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    ProblemPath(path): ProblemPath<RepositoryPath>,
    headers: HeaderMap,
    query: StrictQuery,
) -> Response {
    if !valid_repository_path(&path) {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }
    let (limit, cursor) = match page_query(
        query,
        "listGitHubRepositoryActivity",
        &path.owner,
        Some(&path.repo),
    ) {
        Ok(value) => value,
        Err(code) => return problem_response(code, &headers),
    };
    let pagination = cursor.as_ref().map(|cursor| GitHubPagination::Activity {
        direction: cursor.direction,
        value: cursor.value.clone(),
    });
    if pagination
        .as_ref()
        .is_some_and(|value| !valid_activity_pagination(value))
    {
        return problem_response(ProblemCode::InvalidRequest, &headers);
    }
    match state
        .github_service
        .list_activity(&path.owner, &path.repo, limit, pagination)
        .await
    {
        Ok(page) => activity_page_response(format, &path, limit, page)
            .unwrap_or_else(|()| problem_response(ProblemCode::GithubUpstream, &headers)),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}/languages",
    operation_id = "listGitHubRepositoryLanguages",
    tag = "GitHub",
    security(()),
    params(
        ("owner" = String, Path, min_length = 1, max_length = 39),
        ("repo" = String, Path, min_length = 1, max_length = 100)
    ),
    responses(
        (status = 200, description = "Repository languages", content((GitHubLanguages = "application/json"), (GitHubLanguages = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 429, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 502, response = ProblemResponse),
        (status = 504, response = ProblemResponse)
    )
)]
pub async fn get_github_repo_languages_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    _query: NoQuery,
    ProblemPath(path): ProblemPath<RepositoryPath>,
    headers: HeaderMap,
) -> Response {
    if !valid_repository_path(&path) {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }
    match state
        .github_service
        .list_languages(&path.owner, &path.repo)
        .await
    {
        Ok(languages) => success_response(StatusCode::OK, format, &GitHubLanguages { languages }),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/github/repos/{owner}/{repo}/tags",
    operation_id = "listGitHubRepositoryTags",
    tag = "GitHub",
    security(()),
    params(
        ("owner" = String, Path, min_length = 1, max_length = 39),
        ("repo" = String, Path, min_length = 1, max_length = 100),
        ("limit" = Option<u16>, Query, minimum = 1, maximum = 100, description = "Default 20; closed query"),
        ("cursor" = Option<String>, Query, max_length = 2048, description = "Opaque scoped cursor; closed query")
    ),
    responses(
        (status = 200, description = "Repository tags", headers(("Link" = String, description = "Optional RFC 8288 navigation")), content((GitHubTagPage = "application/json"), (GitHubTagPage = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 429, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 502, response = ProblemResponse),
        (status = 504, response = ProblemResponse)
    )
)]
pub async fn list_github_repo_tags_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    ProblemPath(path): ProblemPath<RepositoryPath>,
    headers: HeaderMap,
    query: StrictQuery,
) -> Response {
    if !valid_repository_path(&path) {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    }
    let (limit, cursor) = match page_query(
        query,
        "listGitHubRepositoryTags",
        &path.owner,
        Some(&path.repo),
    ) {
        Ok(value) => value,
        Err(code) => return problem_response(code, &headers),
    };
    let Ok(pagination) = numbered_pagination(cursor.as_ref()) else {
        return problem_response(ProblemCode::InvalidRequest, &headers);
    };
    match state
        .github_service
        .list_tags(&path.owner, &path.repo, limit, pagination)
        .await
    {
        Ok(page) => tag_page_response(format, &path, limit, page)
            .unwrap_or_else(|()| problem_response(ProblemCode::GithubUpstream, &headers)),
        Err(error) => map_service_error(&headers, error),
    }
}

fn valid_repository_path(path: &RepositoryPath) -> bool {
    valid_github_owner(&path.owner) && valid_github_repository(&path.repo)
}

fn page_query(
    query: StrictQuery,
    operation: &str,
    owner: &str,
    repository: Option<&str>,
) -> Result<(u16, Option<Cursor>), ProblemCode> {
    let query = query
        .closed(&["limit", "cursor"])
        .map_err(|_| ProblemCode::InvalidRequest)?;
    let limit = resolve_limit(query.get("limit"), DEFAULT_LIMIT, MAX_LIMIT)
        .ok_or(ProblemCode::ValidationFailed)?;
    let cursor = query
        .get("cursor")
        .map(decode_cursor)
        .transpose()
        .map_err(|_| ProblemCode::InvalidRequest)?;
    let scope = CursorScope {
        operation,
        owner: Some(owner),
        repository,
        limit,
        category: None,
    };
    if cursor
        .as_ref()
        .is_some_and(|cursor| !cursor.belongs_to(&scope))
    {
        return Err(ProblemCode::InvalidRequest);
    }
    Ok((limit, cursor))
}

fn numbered_pagination(cursor: Option<&Cursor>) -> Result<Option<GitHubPagination>, ()> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let page = canonical_page(&cursor.value).ok_or(())?;
    if cursor.direction == CursorDirection::Next && page == 1 {
        return Err(());
    }
    Ok(Some(GitHubPagination::Numbered(page)))
}

fn canonical_page(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|value| (1..=SAFE_INTEGER_MAX).contains(value))
}

fn valid_activity_pagination(value: &GitHubPagination) -> bool {
    matches!(
        value,
        GitHubPagination::Activity { value, .. }
            if !value.is_empty()
                && value.chars().count() <= 2_048
                && value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
    )
}

fn repository_page_response(
    format: ResponseFormat,
    owner: &str,
    limit: u16,
    page: ProviderPage<RepositorySummary>,
) -> Result<Response, ()> {
    let scope = CursorScope {
        operation: "listGitHubOwnerRepositories",
        owner: Some(owner),
        repository: None,
        limit,
        category: None,
    };
    paged_response(
        format,
        "/v1/github/owners/",
        &format!("{owner}/repos"),
        &scope,
        page.next.as_deref(),
        page.prev.as_deref(),
        &GitHubRepositoryPage {
            count: page.items.len() as u64,
            repos: page.items,
        },
    )
}

fn activity_page_response(
    format: ResponseFormat,
    path: &RepositoryPath,
    limit: u16,
    page: ProviderPage<Activity>,
) -> Result<Response, ()> {
    let scope = CursorScope {
        operation: "listGitHubRepositoryActivity",
        owner: Some(&path.owner),
        repository: Some(&path.repo),
        limit,
        category: None,
    };
    paged_response(
        format,
        "/v1/github/repos/",
        &format!("{}/{}/activity", path.owner, path.repo),
        &scope,
        page.next.as_deref(),
        page.prev.as_deref(),
        &GitHubActivityPage {
            count: page.items.len() as u64,
            activities: page.items,
        },
    )
}

fn tag_page_response(
    format: ResponseFormat,
    path: &RepositoryPath,
    limit: u16,
    page: ProviderPage<Tag>,
) -> Result<Response, ()> {
    let scope = CursorScope {
        operation: "listGitHubRepositoryTags",
        owner: Some(&path.owner),
        repository: Some(&path.repo),
        limit,
        category: None,
    };
    paged_response(
        format,
        "/v1/github/repos/",
        &format!("{}/{}/tags", path.owner, path.repo),
        &scope,
        page.next.as_deref(),
        page.prev.as_deref(),
        &GitHubTagPage {
            count: page.items.len() as u64,
            tags: page.items,
        },
    )
}

fn paged_response<T: Serialize>(
    format: ResponseFormat,
    prefix: &str,
    suffix: &str,
    scope: &CursorScope<'_>,
    next: Option<&str>,
    prev: Option<&str>,
    body: &T,
) -> Result<Response, ()> {
    let next = next
        .map(|value| public_cursor(scope, CursorDirection::Next, value))
        .transpose()?;
    let prev = prev
        .map(|value| public_cursor(scope, CursorDirection::Prev, value))
        .transpose()?;
    let base = format!("{prefix}{suffix}");
    let limit = scope.limit.to_string();
    let link = build_link_header(
        &base,
        &[("limit", limit.as_str())],
        next.as_deref(),
        prev.as_deref(),
    );
    let headers = if link.is_empty() {
        Vec::new()
    } else {
        vec![(
            header::LINK,
            HeaderValue::from_str(&link).expect("local link should be a valid field value"),
        )]
    };
    Ok(success_response_with_headers(
        StatusCode::OK,
        format,
        body,
        headers,
    ))
}

fn public_cursor(
    scope: &CursorScope<'_>,
    direction: CursorDirection,
    value: &str,
) -> Result<String, ()> {
    let encoded = Cursor::new(scope, direction, value).encode();
    validate_cursor_text(&encoded).map_err(|_| ())?;
    Ok(encoded)
}

fn map_service_error(headers: &HeaderMap, error: GitHubServiceError) -> Response {
    match error {
        GitHubServiceError::NotFound => problem_response(ProblemCode::GithubNotFound, headers),
        GitHubServiceError::RateLimited(rate) => {
            let mut response = problem_response(ProblemCode::GithubRateLimit, headers);
            response.headers_mut().insert(
                header::RETRY_AFTER,
                HeaderValue::from_str(&rate.retry_after).expect("validated retry delay"),
            );
            if let Some(reset) = rate.rate_limit_reset {
                response.headers_mut().insert(
                    HeaderName::from_static("x-ratelimit-reset"),
                    HeaderValue::from_str(&reset).expect("validated reset epoch"),
                );
            }
            response
        }
        GitHubServiceError::Timeout => problem_response(ProblemCode::GithubTimeout, headers),
        GitHubServiceError::Upstream(error) => {
            tracing::warn!(reason = ?error.kind, "GitHub operation failed");
            problem_response(ProblemCode::GithubUpstream, headers)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_page, valid_activity_pagination};
    use crate::{pagination::cursor::CursorDirection, services::github::GitHubPagination};

    #[test]
    fn decoded_provider_pagination_values_are_revalidated_before_a_fetch() {
        assert_eq!(canonical_page("1"), Some(1));
        assert_eq!(
            canonical_page("9007199254740991"),
            Some(9_007_199_254_740_991)
        );
        for value in ["", "0", "01", "+1", "9007199254740992"] {
            assert_eq!(canonical_page(value), None);
        }
        assert!(valid_activity_pagination(&GitHubPagination::Activity {
            direction: CursorDirection::Next,
            value: "cursor-value".to_owned(),
        }));
        assert!(!valid_activity_pagination(&GitHubPagination::Activity {
            direction: CursorDirection::Next,
            value: "contains space".to_owned(),
        }));
    }
}
