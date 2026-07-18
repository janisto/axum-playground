use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

use reqwest::{Client, StatusCode};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, Error as _},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use utoipa::ToSchema;

const DEFAULT_BASE_URL: &str = "https://api.github.com";
const DEFAULT_USER_AGENT: &str = "axum-playground/0.1.0";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";
const GITHUB_API_VERSION: &str = "2026-03-10";

#[derive(Clone, Debug)]
pub struct GitHubService {
    inner: Arc<GitHubServiceInner>,
}

#[derive(Clone, Debug)]
enum GitHubServiceInner {
    Http(HttpGitHubService),
    Mock(Box<MockGitHubService>),
}

#[derive(Clone, Debug)]
struct HttpGitHubService {
    transport: GitHubTransport,
    base_url: String,
    token: Option<String>,
}

#[derive(Clone, Debug)]
enum GitHubTransport {
    Http(Client),
    #[cfg(test)]
    Mock(MockGitHubTransport),
}

struct GitHubHttpRequest {
    url: reqwest::Url,
    headers: reqwest::header::HeaderMap,
    bearer_token: Option<String>,
}

#[derive(Debug)]
struct GitHubHttpResponse {
    status: StatusCode,
    headers: reqwest::header::HeaderMap,
    body: Vec<u8>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct MockGitHubTransport {
    responses: Arc<
        std::sync::Mutex<
            std::collections::VecDeque<Result<GitHubHttpResponse, GitHubServiceError>>,
        >,
    >,
    requests: Arc<std::sync::Mutex<Vec<RecordedGitHubRequest>>>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct RecordedGitHubRequest {
    url: reqwest::Url,
    headers: reqwest::header::HeaderMap,
    has_bearer_token: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MockGitHubService {
    owner: Option<Owner>,
    repos_page: ListPage<RepoSummary>,
    repo: Option<Repo>,
    activity_page: ActivityPage,
    languages: Vec<Language>,
    tags_page: ListPage<Tag>,
    error: Option<GitHubServiceError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListPage<T> {
    pub items: Vec<T>,
    pub next_cursor: String,
}

impl<T> Default for ListPage<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct Owner {
    pub login: String,
    pub name: String,
    #[serde(rename = "avatarUrl")]
    pub avatar_url: String,
    #[serde(rename = "htmlUrl")]
    pub html_url: String,
    pub bio: String,
    pub location: String,
    pub blog: String,
    pub company: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct RepoSummary {
    pub name: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    pub description: String,
    #[serde(rename = "htmlUrl")]
    pub html_url: String,
    pub language: String,
    pub stars: i32,
    pub forks: i32,
    #[serde(rename = "openIssues")]
    pub open_issues: i32,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct Repo {
    #[serde(flatten)]
    pub repo_summary: RepoSummary,
    #[serde(rename = "defaultBranch")]
    pub default_branch: String,
    pub license: String,
    pub topics: Vec<String>,
    pub archived: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct ActivityPage {
    pub activities: Vec<Activity>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct Activity {
    pub id: i64,
    pub actor: Option<String>,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub timestamp: String,
    #[serde(rename = "activityType")]
    pub activity_type: String,
    #[serde(rename = "actorAvatarUrl")]
    pub actor_avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct Tag {
    pub name: String,
    pub commit: TagCommit,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct TagCommit {
    pub sha: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
pub struct Language {
    pub name: String,
    pub bytes: i64,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum GitHubServiceError {
    #[error("GitHub resource not found")]
    NotFound,
    #[error("GitHub access forbidden")]
    Forbidden,
    #[error("GitHub rate limit exceeded")]
    RateLimited,
    #[error(transparent)]
    Upstream(GitHubUpstreamError),
}

#[derive(Clone)]
pub struct GitHubUpstreamError {
    pub kind: GitHubUpstreamErrorKind,
    pub status: u16,
    pub retry_after: Option<String>,
    pub rate_limit_reset: Option<String>,
    source: Option<Arc<dyn Error + Send + Sync>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubUpstreamErrorKind {
    NotFound,
    Forbidden,
    RateLimited,
    Upstream,
}

impl GitHubService {
    #[must_use]
    pub fn http(token: Option<String>) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Http(HttpGitHubService {
                transport: GitHubTransport::Http(Client::new()),
                base_url: DEFAULT_BASE_URL.to_owned(),
                token,
            })),
        }
    }

    #[must_use]
    pub fn mock(mock: MockGitHubService) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Mock(Box::new(mock))),
        }
    }

    #[cfg(test)]
    pub(crate) fn http_with_base_url(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Http(HttpGitHubService {
                transport: GitHubTransport::Http(Client::new()),
                base_url: base_url.into(),
                token,
            })),
        }
    }

    #[cfg(test)]
    fn http_with_mock_transport(transport: MockGitHubTransport, token: Option<String>) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Http(HttpGitHubService {
                transport: GitHubTransport::Mock(transport),
                base_url: "https://api.github.test".to_owned(),
                token,
            })),
        }
    }

    pub async fn get_owner(&self, owner: &str) -> Result<Owner, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.get_owner(owner).await,
            GitHubServiceInner::Mock(service) => service.get_owner(owner).await,
        }
    }

    pub async fn list_repos(
        &self,
        owner: &str,
        limit: usize,
        page: &str,
    ) -> Result<ListPage<RepoSummary>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.list_repos(owner, limit, page).await,
            GitHubServiceInner::Mock(service) => service.list_repos(owner, limit, page).await,
        }
    }

    pub async fn get_repo(&self, owner: &str, repo: &str) -> Result<Repo, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.get_repo(owner, repo).await,
            GitHubServiceInner::Mock(service) => service.get_repo(owner, repo).await,
        }
    }

    pub async fn list_activity(
        &self,
        owner: &str,
        repo: &str,
        limit: usize,
        after_cursor: &str,
    ) -> Result<ActivityPage, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => {
                service
                    .list_activity(owner, repo, limit, after_cursor)
                    .await
            }
            GitHubServiceInner::Mock(service) => {
                service
                    .list_activity(owner, repo, limit, after_cursor)
                    .await
            }
        }
    }

    pub async fn list_languages(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Language>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.list_languages(owner, repo).await,
            GitHubServiceInner::Mock(service) => service.list_languages(owner, repo).await,
        }
    }

    pub async fn list_tags(
        &self,
        owner: &str,
        repo: &str,
        limit: usize,
        page: &str,
    ) -> Result<ListPage<Tag>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.list_tags(owner, repo, limit, page).await,
            GitHubServiceInner::Mock(service) => service.list_tags(owner, repo, limit, page).await,
        }
    }
}

impl MockGitHubService {
    #[must_use]
    pub fn demo() -> Self {
        Self {
            owner: Some(Owner {
                login: "octocat".to_owned(),
                name: "The Octocat".to_owned(),
                avatar_url: "https://avatars.githubusercontent.com/u/583231".to_owned(),
                html_url: "https://github.com/octocat".to_owned(),
                bio: String::new(),
                location: "San Francisco".to_owned(),
                blog: "https://github.blog".to_owned(),
                company: "@github".to_owned(),
                created_at: "2011-01-25T18:44:36Z".to_owned(),
                updated_at: "2024-06-01T00:00:00Z".to_owned(),
            }),
            repos_page: ListPage {
                items: vec![RepoSummary {
                    name: "git-consortium".to_owned(),
                    full_name: "octocat/git-consortium".to_owned(),
                    description: "This repo is for demonstration purposes.".to_owned(),
                    html_url: "https://github.com/octocat/git-consortium".to_owned(),
                    language: "Ruby".to_owned(),
                    stars: 16,
                    forks: 10,
                    open_issues: 0,
                    created_at: "2011-01-25T18:44:36Z".to_owned(),
                    updated_at: "2024-06-01T00:00:00Z".to_owned(),
                }],
                next_cursor: String::new(),
            },
            repo: Some(Repo {
                repo_summary: RepoSummary {
                    name: "git-consortium".to_owned(),
                    full_name: "octocat/git-consortium".to_owned(),
                    description: "This repo is for demonstration purposes.".to_owned(),
                    html_url: "https://github.com/octocat/git-consortium".to_owned(),
                    language: "Ruby".to_owned(),
                    stars: 16,
                    forks: 10,
                    open_issues: 0,
                    created_at: "2011-01-25T18:44:36Z".to_owned(),
                    updated_at: "2024-06-01T00:00:00Z".to_owned(),
                },
                default_branch: "master".to_owned(),
                license: "MIT License".to_owned(),
                topics: Vec::new(),
                archived: false,
                disabled: false,
            }),
            activity_page: ActivityPage {
                activities: vec![Activity {
                    id: 1,
                    actor: Some("octocat".to_owned()),
                    git_ref: "refs/heads/master".to_owned(),
                    timestamp: "2024-01-15T10:30:00Z".to_owned(),
                    activity_type: "push".to_owned(),
                    actor_avatar_url: Some(
                        "https://avatars.githubusercontent.com/u/583231".to_owned(),
                    ),
                }],
                next_cursor: String::new(),
            },
            languages: vec![Language {
                name: "Ruby".to_owned(),
                bytes: 6789,
            }],
            tags_page: ListPage {
                items: vec![Tag {
                    name: "v1.0".to_owned(),
                    commit: TagCommit {
                        sha: "abc123".to_owned(),
                    },
                }],
                next_cursor: String::new(),
            },
            error: None,
        }
    }

    #[must_use]
    pub fn with_error(mut self, error: GitHubServiceError) -> Self {
        self.error = Some(error);
        self
    }

    #[must_use]
    pub fn with_activity_page(mut self, activity_page: ActivityPage) -> Self {
        self.activity_page = activity_page;
        self
    }

    #[must_use]
    pub fn with_repos_page(mut self, repos_page: ListPage<RepoSummary>) -> Self {
        self.repos_page = repos_page;
        self
    }

    #[must_use]
    pub fn with_tags_page(mut self, tags_page: ListPage<Tag>) -> Self {
        self.tags_page = tags_page;
        self
    }

    async fn get_owner(&self, owner: &str) -> Result<Owner, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        self.owner
            .clone()
            .filter(|current_owner| current_owner.login == owner)
            .ok_or(GitHubServiceError::NotFound)
    }

    async fn list_repos(
        &self,
        owner: &str,
        _limit: usize,
        _page: &str,
    ) -> Result<ListPage<RepoSummary>, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        if self
            .owner
            .as_ref()
            .is_some_and(|current_owner| current_owner.login == owner)
        {
            Ok(self.repos_page.clone())
        } else {
            Err(GitHubServiceError::NotFound)
        }
    }

    async fn get_repo(&self, owner: &str, repo: &str) -> Result<Repo, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        self.repo
            .clone()
            .filter(|current_repo| current_repo.repo_summary.full_name == format!("{owner}/{repo}"))
            .ok_or(GitHubServiceError::NotFound)
    }

    async fn list_activity(
        &self,
        owner: &str,
        repo: &str,
        _limit: usize,
        _after_cursor: &str,
    ) -> Result<ActivityPage, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        if self.repo.as_ref().is_some_and(|current_repo| {
            current_repo.repo_summary.full_name == format!("{owner}/{repo}")
        }) {
            Ok(self.activity_page.clone())
        } else {
            Err(GitHubServiceError::NotFound)
        }
    }

    async fn list_languages(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Language>, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        if self.repo.as_ref().is_some_and(|current_repo| {
            current_repo.repo_summary.full_name == format!("{owner}/{repo}")
        }) {
            Ok(self.languages.clone())
        } else {
            Err(GitHubServiceError::NotFound)
        }
    }

    async fn list_tags(
        &self,
        owner: &str,
        repo: &str,
        _limit: usize,
        _page: &str,
    ) -> Result<ListPage<Tag>, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        if self.repo.as_ref().is_some_and(|current_repo| {
            current_repo.repo_summary.full_name == format!("{owner}/{repo}")
        }) {
            Ok(self.tags_page.clone())
        } else {
            Err(GitHubServiceError::NotFound)
        }
    }
}

impl HttpGitHubService {
    async fn get_owner(&self, owner: &str) -> Result<Owner, GitHubServiceError> {
        let payload: GitHubOwnerPayload = self.send_json(self.url(&["users", owner])).await?;
        Ok(payload.into_owner())
    }

    async fn list_repos(
        &self,
        owner: &str,
        limit: usize,
        page: &str,
    ) -> Result<ListPage<RepoSummary>, GitHubServiceError> {
        let url = self.paginated_url(&["users", owner, "repos"], limit, "page", page);
        let response = self.send(url).await?;
        let next_cursor = response
            .headers
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(|link| extract_next_query_value(link, "page"));
        let payload = decode_json::<Vec<GitHubRepoSummaryPayload>>(&response)?;
        Ok(ListPage {
            items: payload
                .into_iter()
                .map(GitHubRepoSummaryPayload::into_repo_summary)
                .collect(),
            next_cursor: next_cursor.unwrap_or_default(),
        })
    }

    async fn get_repo(&self, owner: &str, repo: &str) -> Result<Repo, GitHubServiceError> {
        let payload: GitHubRepoPayload = self.send_json(self.url(&["repos", owner, repo])).await?;
        Ok(payload.into_repo())
    }

    async fn list_activity(
        &self,
        owner: &str,
        repo: &str,
        limit: usize,
        after_cursor: &str,
    ) -> Result<ActivityPage, GitHubServiceError> {
        let url = self.paginated_url(
            &["repos", owner, repo, "activity"],
            limit,
            "after",
            after_cursor,
        );
        let response = self.send(url).await?;
        let next_cursor = response
            .headers
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(|link| extract_next_query_value(link, "after"));
        let payload = decode_json::<Vec<GitHubActivityPayload>>(&response)?;

        Ok(ActivityPage {
            activities: payload
                .into_iter()
                .map(GitHubActivityPayload::into_activity)
                .collect(),
            next_cursor: next_cursor.unwrap_or_default(),
        })
    }

    async fn list_languages(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Language>, GitHubServiceError> {
        let payload: BTreeMap<String, i64> = self
            .send_json(self.url(&["repos", owner, repo, "languages"]))
            .await?;

        let mut languages = payload
            .into_iter()
            .map(|(name, bytes)| Language { name, bytes })
            .collect::<Vec<_>>();
        languages.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(languages)
    }

    async fn list_tags(
        &self,
        owner: &str,
        repo: &str,
        limit: usize,
        page: &str,
    ) -> Result<ListPage<Tag>, GitHubServiceError> {
        let url = self.paginated_url(&["repos", owner, repo, "tags"], limit, "page", page);
        let response = self.send(url).await?;
        let next_cursor = response
            .headers
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(|link| extract_next_query_value(link, "page"));
        let payload = decode_json::<Vec<GitHubTagPayload>>(&response)?;

        Ok(ListPage {
            items: payload
                .into_iter()
                .map(GitHubTagPayload::into_tag)
                .collect(),
            next_cursor: next_cursor.unwrap_or_default(),
        })
    }

    async fn send_json<T>(&self, url: reqwest::Url) -> Result<T, GitHubServiceError>
    where
        T: DeserializeOwned,
    {
        let response = self.send(url).await?;
        decode_json(&response)
    }

    async fn send(&self, url: reqwest::Url) -> Result<GitHubHttpResponse, GitHubServiceError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static(GITHUB_ACCEPT),
        );
        headers.insert(
            reqwest::header::HeaderName::from_static("x-github-api-version"),
            reqwest::header::HeaderValue::from_static(GITHUB_API_VERSION),
        );
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static(DEFAULT_USER_AGENT),
        );
        let response = self
            .transport
            .execute(GitHubHttpRequest {
                url,
                headers,
                bearer_token: self.token.clone(),
            })
            .await?;

        if response.status.is_success() {
            Ok(response)
        } else {
            Err(map_http_error(response.status, &response.headers))
        }
    }

    fn paginated_url(
        &self,
        segments: &[&str],
        limit: usize,
        cursor_name: &str,
        cursor: &str,
    ) -> reqwest::Url {
        let mut url = self.url(segments);
        let limit = limit.to_string();
        let mut query = url.query_pairs_mut();
        query.append_pair("per_page", &limit);
        if !cursor.is_empty() {
            query.append_pair(cursor_name, cursor);
        }
        drop(query);
        url
    }

    fn url(&self, segments: &[&str]) -> reqwest::Url {
        let mut url =
            reqwest::Url::parse(&self.base_url).expect("GitHub base URL should be an absolute URL");
        url.path_segments_mut()
            .expect("GitHub base URL should support path segments")
            .pop_if_empty()
            .extend(segments);
        url
    }
}

impl GitHubTransport {
    async fn execute(
        &self,
        request: GitHubHttpRequest,
    ) -> Result<GitHubHttpResponse, GitHubServiceError> {
        match self {
            Self::Http(client) => {
                let request = build_github_request(client, request)?;
                let response = client.execute(request).await.map_err(|error| {
                    GitHubServiceError::Upstream(
                        GitHubUpstreamError::upstream(0).with_source(error),
                    )
                })?;
                let status = response.status();
                let headers = response.headers().clone();
                let body = if status.is_success() {
                    response
                        .bytes()
                        .await
                        .map_err(|error| {
                            GitHubServiceError::Upstream(
                                GitHubUpstreamError::upstream(status.as_u16()).with_source(error),
                            )
                        })?
                        .to_vec()
                } else {
                    Vec::new()
                };
                Ok(GitHubHttpResponse {
                    status,
                    headers,
                    body,
                })
            }
            #[cfg(test)]
            Self::Mock(transport) => transport.execute(request),
        }
    }
}

fn build_github_request(
    client: &Client,
    request: GitHubHttpRequest,
) -> Result<reqwest::Request, GitHubServiceError> {
    let mut builder = client
        .get(request.url)
        .headers(request.headers)
        .timeout(std::time::Duration::from_secs(10));
    if let Some(token) = request.bearer_token {
        builder = builder.bearer_auth(token);
    }
    builder.build().map_err(|error| {
        GitHubServiceError::Upstream(GitHubUpstreamError::upstream(0).with_source(error))
    })
}

#[cfg(test)]
impl MockGitHubTransport {
    fn new(responses: Vec<Result<GitHubHttpResponse, GitHubServiceError>>) -> Self {
        Self {
            responses: Arc::new(std::sync::Mutex::new(responses.into())),
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn execute(
        &self,
        request: GitHubHttpRequest,
    ) -> Result<GitHubHttpResponse, GitHubServiceError> {
        self.requests
            .lock()
            .expect("mock GitHub requests should not be poisoned")
            .push(RecordedGitHubRequest {
                url: request.url,
                headers: request.headers,
                has_bearer_token: request.bearer_token.is_some(),
            });
        self.responses
            .lock()
            .expect("mock GitHub responses should not be poisoned")
            .pop_front()
            .expect("mock GitHub response should be configured")
    }

    fn requests(&self) -> Vec<RecordedGitHubRequest> {
        self.requests
            .lock()
            .expect("mock GitHub requests should not be poisoned")
            .clone()
    }
}

impl GitHubUpstreamError {
    #[must_use]
    pub fn new(
        kind: GitHubUpstreamErrorKind,
        status: u16,
        retry_after: Option<String>,
        rate_limit_reset: Option<String>,
    ) -> Self {
        Self {
            kind,
            status,
            retry_after,
            rate_limit_reset,
            source: None,
        }
    }

    fn upstream(status: u16) -> Self {
        Self::new(GitHubUpstreamErrorKind::Upstream, status, None, None)
    }

    fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Arc::new(source));
        self
    }
}

impl fmt::Debug for GitHubUpstreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitHubUpstreamError")
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("retry_after", &self.retry_after)
            .field("rate_limit_reset", &self.rate_limit_reset)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for GitHubUpstreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "GitHub upstream {:?} error with status {}",
            self.kind, self.status
        )
    }
}

impl Error for GitHubUpstreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

fn map_http_error(status: StatusCode, headers: &reqwest::header::HeaderMap) -> GitHubServiceError {
    let retry_after = header_value(headers, reqwest::header::RETRY_AFTER.as_str());
    let rate_limit_reset = header_value(headers, "x-ratelimit-reset");
    let rate_limit_remaining = header_value(headers, "x-ratelimit-remaining");

    if status == StatusCode::NOT_FOUND {
        return GitHubServiceError::Upstream(GitHubUpstreamError::new(
            GitHubUpstreamErrorKind::NotFound,
            status.as_u16(),
            retry_after,
            rate_limit_reset,
        ));
    }

    if status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (retry_after.is_some() || rate_limit_remaining.as_deref() == Some("0")))
    {
        return GitHubServiceError::Upstream(GitHubUpstreamError::new(
            GitHubUpstreamErrorKind::RateLimited,
            status.as_u16(),
            retry_after,
            rate_limit_reset,
        ));
    }

    if status == StatusCode::FORBIDDEN {
        return GitHubServiceError::Upstream(GitHubUpstreamError::new(
            GitHubUpstreamErrorKind::Forbidden,
            status.as_u16(),
            retry_after,
            rate_limit_reset,
        ));
    }

    GitHubServiceError::Upstream(GitHubUpstreamError::new(
        GitHubUpstreamErrorKind::Upstream,
        status.as_u16(),
        retry_after,
        rate_limit_reset,
    ))
}

fn header_value(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn decode_json<T>(response: &GitHubHttpResponse) -> Result<T, GitHubServiceError>
where
    T: DeserializeOwned,
{
    let status = response.status.as_u16();
    serde_json::from_slice::<T>(&response.body).map_err(|error| {
        GitHubServiceError::Upstream(GitHubUpstreamError::upstream(status).with_source(error))
    })
}

fn deserialize_http_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let url = reqwest::Url::parse(&value).map_err(D::Error::custom)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(D::Error::custom("expected an absolute HTTP(S) URL"));
    }
    Ok(value)
}

fn deserialize_rfc3339<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    OffsetDateTime::parse(&value, &Rfc3339).map_err(D::Error::custom)?;
    Ok(value)
}

fn extract_next_query_value(link_header: &str, query_key: &str) -> Option<String> {
    link_header.split(',').find_map(|part| {
        let part = part.trim();
        if !part.contains("rel=\"next\"") {
            return None;
        }

        let start = part.find('<')? + 1;
        let end = part[start..].find('>')? + start;
        let url = reqwest::Url::parse(&part[start..end]).ok()?;
        url.query_pairs()
            .find(|(key, _)| key == query_key)
            .map(|(_, value)| value.into_owned())
    })
}

#[derive(Debug, Deserialize)]
struct GitHubOwnerPayload {
    login: String,
    name: Option<String>,
    #[serde(deserialize_with = "deserialize_http_url")]
    avatar_url: String,
    #[serde(deserialize_with = "deserialize_http_url")]
    html_url: String,
    bio: Option<String>,
    location: Option<String>,
    blog: Option<String>,
    company: Option<String>,
    #[serde(deserialize_with = "deserialize_rfc3339")]
    created_at: String,
    #[serde(deserialize_with = "deserialize_rfc3339")]
    updated_at: String,
}

impl GitHubOwnerPayload {
    fn into_owner(self) -> Owner {
        Owner {
            login: self.login,
            name: self.name.unwrap_or_default(),
            avatar_url: self.avatar_url,
            html_url: self.html_url,
            bio: self.bio.unwrap_or_default(),
            location: self.location.unwrap_or_default(),
            blog: self.blog.unwrap_or_default(),
            company: self.company.unwrap_or_default(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRepoSummaryPayload {
    name: String,
    full_name: String,
    description: Option<String>,
    #[serde(deserialize_with = "deserialize_http_url")]
    html_url: String,
    language: Option<String>,
    stargazers_count: i32,
    forks_count: i32,
    open_issues_count: i32,
    #[serde(deserialize_with = "deserialize_rfc3339")]
    created_at: String,
    #[serde(deserialize_with = "deserialize_rfc3339")]
    updated_at: String,
}

impl GitHubRepoSummaryPayload {
    fn into_repo_summary(self) -> RepoSummary {
        RepoSummary {
            name: self.name,
            full_name: self.full_name,
            description: self.description.unwrap_or_default(),
            html_url: self.html_url,
            language: self.language.unwrap_or_default(),
            stars: self.stargazers_count,
            forks: self.forks_count,
            open_issues: self.open_issues_count,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubLicensePayload {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRepoPayload {
    #[serde(flatten)]
    summary: GitHubRepoSummaryPayload,
    default_branch: String,
    license: Option<GitHubLicensePayload>,
    topics: Option<Vec<String>>,
    archived: bool,
    disabled: bool,
}

impl GitHubRepoPayload {
    fn into_repo(self) -> Repo {
        Repo {
            repo_summary: self.summary.into_repo_summary(),
            default_branch: self.default_branch,
            license: self.license.map(|license| license.name).unwrap_or_default(),
            topics: self.topics.unwrap_or_default(),
            archived: self.archived,
            disabled: self.disabled,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubActivityPayload {
    id: i64,
    actor: Option<GitHubActivityActorPayload>,
    #[serde(rename = "ref")]
    git_ref: String,
    #[serde(deserialize_with = "deserialize_rfc3339")]
    timestamp: String,
    activity_type: String,
}

#[derive(Debug, Deserialize)]
struct GitHubActivityActorPayload {
    login: String,
    #[serde(deserialize_with = "deserialize_http_url")]
    avatar_url: String,
}

impl GitHubActivityPayload {
    fn into_activity(self) -> Activity {
        let (actor, actor_avatar_url) = self
            .actor
            .map(|actor| (Some(actor.login), Some(actor.avatar_url)))
            .unwrap_or((None, None));

        Activity {
            id: self.id,
            actor,
            git_ref: self.git_ref,
            timestamp: self.timestamp,
            activity_type: self.activity_type,
            actor_avatar_url,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubTagPayload {
    name: String,
    commit: GitHubTagCommitPayload,
}

impl GitHubTagPayload {
    fn into_tag(self) -> Tag {
        Tag {
            name: self.name,
            commit: self.commit.into_tag_commit(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubTagCommitPayload {
    sha: String,
}

impl GitHubTagCommitPayload {
    fn into_tag_commit(self) -> TagCommit {
        TagCommit { sha: self.sha }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use serde_json::{Value, json};

    use super::{
        GITHUB_ACCEPT, GitHubActivityPayload, GitHubHttpRequest, GitHubHttpResponse,
        GitHubOwnerPayload, GitHubRepoPayload, GitHubService, GitHubServiceError,
        GitHubServiceInner, GitHubUpstreamErrorKind, MockGitHubTransport, build_github_request,
        decode_json, map_http_error,
    };

    fn json_response(status: StatusCode, headers: HeaderMap, body: &Value) -> GitHubHttpResponse {
        GitHubHttpResponse {
            status,
            headers,
            body: serde_json::to_vec(&body).expect("mock GitHub body should serialize"),
        }
    }

    #[test]
    fn http_service_encodes_untrusted_path_segments() {
        let service = GitHubService::http_with_base_url("https://api.github.test", None);
        let GitHubServiceInner::Http(service) = service.inner.as_ref() else {
            panic!("expected HTTP service");
        };

        assert_eq!(
            service.url(&["repos", "owner", "repo/name"]).as_str(),
            "https://api.github.test/repos/owner/repo%2Fname"
        );
    }

    #[test]
    fn reqwest_transport_builds_bearer_authenticated_timed_requests() {
        let request = build_github_request(
            &reqwest::Client::new(),
            GitHubHttpRequest {
                url: reqwest::Url::parse("https://api.github.test/users/octocat")
                    .expect("test URL should parse"),
                headers: HeaderMap::new(),
                bearer_token: Some("test-token".to_owned()),
            },
        )
        .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-token")
        );
        assert_eq!(request.timeout(), Some(&std::time::Duration::from_secs(10)));
    }

    #[tokio::test]
    async fn upstream_contract_failure_preserves_successful_http_status() {
        use std::error::Error as _;

        let payload = json!({
            "login": "octocat",
            "name": "The Octocat",
            "avatar_url": "javascript:alert(1)",
            "html_url": "https://github.com/octocat",
            "bio": "",
            "location": "San Francisco",
            "blog": "https://github.blog",
            "company": "@github",
            "created_at": "2011-01-25T18:44:36Z",
            "updated_at": "2024-06-01T00:00:00Z"
        });
        let response = json_response(StatusCode::OK, HeaderMap::new(), &payload);

        let error = decode_json::<GitHubOwnerPayload>(&response)
            .expect_err("non-HTTP avatar URL should violate the upstream contract");

        let GitHubServiceError::Upstream(upstream) = error else {
            panic!("expected upstream error");
        };
        assert_eq!(upstream.kind, GitHubUpstreamErrorKind::Upstream);
        assert_eq!(upstream.status, 200);
        assert_eq!(
            upstream.to_string(),
            "GitHub upstream Upstream error with status 200"
        );
        let debug = format!("{upstream:?}");
        assert!(debug.contains("kind: Upstream"));
        assert!(debug.contains("status: 200"));
        assert!(upstream.source().is_some());
        assert!(!debug.contains("javascript:alert"));
    }

    #[test]
    fn upstream_payloads_reject_invalid_urls_and_timestamps() {
        let repo_with_invalid_url = json!({
            "name": "git-consortium",
            "full_name": "octocat/git-consortium",
            "description": "demo",
            "html_url": "/octocat/git-consortium",
            "language": "Rust",
            "stargazers_count": 1,
            "forks_count": 2,
            "open_issues_count": 3,
            "created_at": "2011-01-25T18:44:36Z",
            "updated_at": "2024-06-01T00:00:00Z",
            "default_branch": "main",
            "license": null,
            "topics": [],
            "archived": false,
            "disabled": false
        });
        assert!(serde_json::from_value::<GitHubRepoPayload>(repo_with_invalid_url).is_err());

        let repo_with_non_http_url = json!({
            "name": "git-consortium",
            "full_name": "octocat/git-consortium",
            "description": "demo",
            "html_url": "ftp://example.com/octocat/git-consortium",
            "language": "Rust",
            "stargazers_count": 1,
            "forks_count": 2,
            "open_issues_count": 3,
            "created_at": "2011-01-25T18:44:36Z",
            "updated_at": "2024-06-01T00:00:00Z",
            "default_branch": "main",
            "license": null,
            "topics": [],
            "archived": false,
            "disabled": false
        });
        assert!(serde_json::from_value::<GitHubRepoPayload>(repo_with_non_http_url).is_err());

        let repo_with_invalid_timestamp = json!({
            "name": "git-consortium",
            "full_name": "octocat/git-consortium",
            "description": "demo",
            "html_url": "https://github.com/octocat/git-consortium",
            "language": "Rust",
            "stargazers_count": 1,
            "forks_count": 2,
            "open_issues_count": 3,
            "created_at": "yesterday",
            "updated_at": "2024-06-01T00:00:00Z",
            "default_branch": "main",
            "license": null,
            "topics": [],
            "archived": false,
            "disabled": false
        });
        assert!(serde_json::from_value::<GitHubRepoPayload>(repo_with_invalid_timestamp).is_err());

        let activity_with_invalid_actor_url = json!({
            "id": 1,
            "actor": { "login": "octocat", "avatar_url": "not a URL" },
            "ref": "refs/heads/main",
            "timestamp": "2024-01-15T10:30:00Z",
            "activity_type": "push"
        });
        assert!(
            serde_json::from_value::<GitHubActivityPayload>(activity_with_invalid_actor_url)
                .is_err()
        );

        let activity_with_invalid_timestamp = json!({
            "id": 1,
            "actor": null,
            "ref": "refs/heads/main",
            "timestamp": "2024-13-99",
            "activity_type": "push"
        });
        assert!(
            serde_json::from_value::<GitHubActivityPayload>(activity_with_invalid_timestamp)
                .is_err()
        );
    }

    #[tokio::test]
    async fn http_service_maps_owner_repository_and_tag_payloads() {
        let mut repos_headers = HeaderMap::new();
        repos_headers.insert(
            header::LINK,
            HeaderValue::from_static(
                "<https://api.github.test/users/octocat/repos?page=4>; rel=\"next\"",
            ),
        );
        let mut tags_headers = HeaderMap::new();
        tags_headers.insert(
            header::LINK,
            HeaderValue::from_static(
                "<https://api.github.test/repos/octocat/git-consortium/tags?page=3>; rel=\"next\"",
            ),
        );
        let transport = MockGitHubTransport::new(vec![
            Ok(json_response(
                StatusCode::OK,
                HeaderMap::new(),
                &json!({
                    "login": "octocat",
                    "name": "The Octocat",
                    "avatar_url": "https://avatars.githubusercontent.com/u/583231",
                    "html_url": "https://github.com/octocat",
                    "bio": "",
                    "location": "San Francisco",
                    "blog": "https://github.blog",
                    "company": "@github",
                    "created_at": "2011-01-25T18:44:36Z",
                    "updated_at": "2024-06-01T00:00:00Z"
                }),
            )),
            Ok(json_response(
                StatusCode::OK,
                repos_headers,
                &json!([{
                    "name": "git-consortium",
                    "full_name": "octocat/git-consortium",
                    "description": null,
                    "html_url": "https://github.com/octocat/git-consortium",
                    "language": null,
                    "stargazers_count": 10,
                    "forks_count": 2,
                    "open_issues_count": 1,
                    "created_at": "2011-01-25T18:44:36Z",
                    "updated_at": "2024-06-01T00:00:00Z"
                }]),
            )),
            Ok(json_response(
                StatusCode::OK,
                tags_headers,
                &json!([{
                    "name": "v1.0.0",
                    "commit": { "sha": "abc123" }
                }]),
            )),
        ]);
        let service =
            GitHubService::http_with_mock_transport(transport.clone(), Some("token".to_owned()));
        let owner = service
            .get_owner("octocat")
            .await
            .expect("owner should load");
        assert_eq!(owner.company, "@github");
        assert_eq!(owner.location, "San Francisco");

        let repos = service
            .list_repos("octocat", 50, "3")
            .await
            .expect("repositories should load");
        assert_eq!(repos.next_cursor, "4");
        assert_eq!(repos.items.len(), 1);
        assert_eq!(repos.items[0].name, "git-consortium");
        assert_eq!(repos.items[0].description, "");

        let tags = service
            .list_tags("octocat", "git-consortium", 25, "2")
            .await
            .expect("tags should load");

        assert_eq!(tags.next_cursor, "3");
        assert_eq!(tags.items.len(), 1);
        assert_eq!(tags.items[0].name, "v1.0.0");
        assert_eq!(tags.items[0].commit.sha, "abc123");

        let requests = transport.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].url.path(), "/users/octocat");
        assert_eq!(
            requests[1]
                .url
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<std::collections::HashMap<_, _>>(),
            std::collections::HashMap::from([
                ("per_page".to_owned(), "50".to_owned()),
                ("page".to_owned(), "3".to_owned()),
            ])
        );
        assert_eq!(
            requests[2]
                .url
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<std::collections::HashMap<_, _>>(),
            std::collections::HashMap::from([
                ("per_page".to_owned(), "25".to_owned()),
                ("page".to_owned(), "2".to_owned()),
            ])
        );
        assert!(requests.iter().all(|request| request.has_bearer_token));
        assert!(requests.iter().all(|request| {
            request
                .headers
                .get(header::ACCEPT)
                .and_then(|value| value.to_str().ok())
                == Some(GITHUB_ACCEPT)
        }));
        for request in &requests {
            assert_eq!(
                request
                    .headers
                    .get("x-github-api-version")
                    .and_then(|value| value.to_str().ok()),
                Some("2026-03-10"),
                "every GitHub request must use the locked API version"
            );
        }
        assert!(
            requests
                .iter()
                .all(|request| request.headers.contains_key(header::USER_AGENT))
        );
    }

    #[test]
    fn http_error_mapping_distinguishes_not_found_forbidden_rate_limit_and_upstream() {
        let headers = HeaderMap::new();
        for (status, expected) in [
            (StatusCode::NOT_FOUND, GitHubUpstreamErrorKind::NotFound),
            (StatusCode::FORBIDDEN, GitHubUpstreamErrorKind::Forbidden),
            (
                StatusCode::TOO_MANY_REQUESTS,
                GitHubUpstreamErrorKind::RateLimited,
            ),
            (StatusCode::BAD_GATEWAY, GitHubUpstreamErrorKind::Upstream),
        ] {
            let GitHubServiceError::Upstream(error) = map_http_error(status, &headers) else {
                panic!("HTTP failures should remain upstream errors");
            };
            assert_eq!(error.kind, expected);
            assert_eq!(error.status, status.as_u16());
        }

        let mut rate_limit_headers = HeaderMap::new();
        rate_limit_headers.insert(
            "x-ratelimit-remaining",
            "0".parse().expect("header value should parse"),
        );
        let GitHubServiceError::Upstream(error) =
            map_http_error(StatusCode::FORBIDDEN, &rate_limit_headers)
        else {
            panic!("HTTP failures should remain upstream errors");
        };
        assert_eq!(error.kind, GitHubUpstreamErrorKind::RateLimited);
    }

    #[tokio::test]
    async fn http_service_maps_rate_limits_and_extracts_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        headers.insert(
            header::HeaderName::from_static("x-ratelimit-reset"),
            HeaderValue::from_static("1700000000"),
        );
        headers.insert(
            header::HeaderName::from_static("x-ratelimit-remaining"),
            HeaderValue::from_static("0"),
        );
        let transport = MockGitHubTransport::new(vec![Ok(json_response(
            StatusCode::FORBIDDEN,
            headers,
            &json!({"message": "rate limited"}),
        ))]);
        let service = GitHubService::http_with_mock_transport(transport, None);
        let error = service
            .get_owner("octocat")
            .await
            .expect_err("request should fail");

        match error {
            GitHubServiceError::Upstream(error) => {
                assert_eq!(error.kind, GitHubUpstreamErrorKind::RateLimited);
                assert_eq!(error.retry_after.as_deref(), Some("60"));
                assert_eq!(error.rate_limit_reset.as_deref(), Some("1700000000"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_service_maps_live_activity_contract_and_extracts_next_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::LINK,
            HeaderValue::from_static(
                "<https://api.github.com/repos/octocat/git-consortium/activity?after=abc123>; rel=\"next\"",
            ),
        );
        let transport = MockGitHubTransport::new(vec![Ok(json_response(
            StatusCode::OK,
            headers,
            &json!([
                {
                    "id": 1,
                    "node_id": "RA_kwDOExample",
                    "actor": {
                        "login": "octocat",
                        "avatar_url": "https://avatars.githubusercontent.com/u/583231",
                        "type": "User"
                    },
                    "ref": "refs/heads/master",
                    "timestamp": "2024-01-15T10:30:00Z",
                    "activity_type": "push",
                    "before": "1111111111111111111111111111111111111111",
                    "after": "2222222222222222222222222222222222222222"
                },
                {
                    "id": 2,
                    "actor": null,
                    "ref": "refs/heads/deleted",
                    "timestamp": "2024-01-15T11:30:00Z",
                    "activity_type": "branch_deletion"
                }
            ]),
        ))]);
        let service = GitHubService::http_with_mock_transport(transport.clone(), None);
        let page = service
            .list_activity("octocat", "git-consortium", 10, "")
            .await
            .expect("activity should load");

        assert_eq!(page.next_cursor, "abc123");
        assert_eq!(page.activities.len(), 2);
        assert_eq!(page.activities[0].id, 1);
        assert_eq!(page.activities[0].actor.as_deref(), Some("octocat"));
        assert_eq!(page.activities[0].git_ref, "refs/heads/master");
        assert_eq!(page.activities[0].timestamp, "2024-01-15T10:30:00Z");
        assert_eq!(page.activities[0].activity_type, "push");
        assert_eq!(
            page.activities[0].actor_avatar_url.as_deref(),
            Some("https://avatars.githubusercontent.com/u/583231")
        );
        assert_eq!(page.activities[1].actor, None);
        assert_eq!(page.activities[1].actor_avatar_url, None);

        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        let query = requests[0]
            .url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(query.get("per_page").map(String::as_str), Some("10"));
        assert!(!query.contains_key("after"));
    }

    #[tokio::test]
    async fn http_service_sorts_languages_by_byte_count() {
        let transport = MockGitHubTransport::new(vec![Ok(json_response(
            StatusCode::OK,
            HeaderMap::new(),
            &json!({"Ruby": 6789, "Go": 12345}),
        ))]);
        let service = GitHubService::http_with_mock_transport(transport, None);
        let languages = service
            .list_languages("octocat", "git-consortium")
            .await
            .expect("languages should load");

        assert_eq!(
            languages.first().map(|language| language.name.as_str()),
            Some("Go")
        );
        assert_eq!(
            languages.first().map(|language| language.bytes),
            Some(12345)
        );
    }
}
