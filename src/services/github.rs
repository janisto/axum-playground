use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use reqwest::{
    Client, StatusCode, Url,
    header::{self, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::{
    http::{
        codec::parse_strict_json,
        extract::decode_query_component,
        negotiation::{decode_parameter, split_outside_quotes, valid_token},
    },
    pagination::cursor::CursorDirection,
    validation::{SAFE_INTEGER_MAX, normalize_timestamp},
};

const DEFAULT_BASE_URL: &str = "https://api.github.com";
const DEFAULT_USER_AGENT: &str = "axum-playground/0.1.0";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";
const GITHUB_API_VERSION: &str = "2026-03-10";
const MAX_GITHUB_RESPONSE_BODY_BYTES: usize = 4_194_304;
const GITHUB_DEADLINE: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 3;

#[derive(Clone, Debug)]
pub struct GitHubService {
    inner: Arc<GitHubServiceInner>,
}

#[derive(Clone, Debug)]
enum GitHubServiceInner {
    Http(HttpGitHubService),
    Mock(Box<MockGitHubService>),
}

#[derive(Clone)]
struct HttpGitHubService {
    transport: GitHubTransport,
    base_url: Url,
    clock: GitHubClock,
}

impl fmt::Debug for HttpGitHubService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpGitHubService")
            .field("base_origin", &self.base_url.origin().ascii_serialization())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
enum GitHubTransport {
    Http(Client),
    #[cfg(test)]
    Mock(MockGitHubTransport),
}

#[derive(Clone, Copy, Debug)]
enum GitHubClock {
    System,
    #[cfg(test)]
    Fixed {
        seconds: u64,
        nanos: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitHubPagination {
    Numbered(u64),
    Activity {
        direction: CursorDirection,
        value: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderPage<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
    pub prev: Option<String>,
}

impl<T> Default for ProviderPage<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next: None,
            prev: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MockGitHubService {
    owner: Option<Owner>,
    repos_page: ProviderPage<RepositorySummary>,
    repository: Option<Repository>,
    activity_page: ProviderPage<Activity>,
    languages: Vec<Language>,
    tags_page: ProviderPage<Tag>,
    error: Option<GitHubServiceError>,
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<MockGitHubCall>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockGitHubCall {
    pub operation: &'static str,
    pub owner: String,
    pub repository: Option<String>,
    pub limit: Option<u16>,
    pub pagination: Option<GitHubPagination>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Owner {
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub id: u64,
    pub login: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub name: Option<String>,
    pub avatar_url: String,
    pub html_url: String,
    pub company: Option<String>,
    pub blog: Option<String>,
    pub location: Option<String>,
    pub bio: Option<String>,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub public_repos: u64,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub followers: u64,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub following: u64,
    #[schema(format = DateTime)]
    pub created_at: String,
    #[schema(format = DateTime)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositorySummary {
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub html_url: String,
    pub fork: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Repository {
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub html_url: String,
    pub fork: bool,
    pub language: Option<String>,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub stargazers_count: u64,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub forks_count: u64,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub open_issues_count: u64,
    pub archived: bool,
    #[schema(format = DateTime)]
    pub created_at: String,
    #[schema(format = DateTime)]
    pub updated_at: String,
    #[schema(format = DateTime)]
    pub pushed_at: Option<String>,
    pub default_branch: String,
    pub license: Option<String>,
    pub topics: Vec<String>,
    pub disabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Activity {
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub id: u64,
    pub actor: Option<String>,
    pub actor_avatar_url: Option<String>,
    #[serde(rename = "ref")]
    pub git_ref: String,
    #[schema(format = DateTime)]
    pub timestamp: String,
    pub activity_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Language {
    pub name: String,
    #[schema(minimum = 0, maximum = 9_007_199_254_740_991_u64)]
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    pub name: String,
    pub commit: TagCommit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TagCommit {
    pub sha: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHubRateLimit {
    pub retry_after: String,
    pub rate_limit_reset: Option<String>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum GitHubServiceError {
    #[error("GitHub resource not found")]
    NotFound,
    #[error("GitHub rate limit exceeded")]
    RateLimited(GitHubRateLimit),
    #[error("GitHub request timed out")]
    Timeout,
    #[error(transparent)]
    Upstream(GitHubUpstreamError),
}

#[derive(Clone)]
pub struct GitHubUpstreamError {
    pub kind: GitHubUpstreamErrorKind,
    source: Option<Arc<dyn Error + Send + Sync>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHubUpstreamErrorKind {
    Transport,
    Redirect,
    Encoding,
    Status,
    Media,
    Size,
    Json,
    Schema,
    Pagination,
}

impl GitHubUpstreamError {
    fn new(kind: GitHubUpstreamErrorKind) -> Self {
        Self { kind, source: None }
    }

    fn with_source(
        kind: GitHubUpstreamErrorKind,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            source: Some(Arc::new(source)),
        }
    }
}

impl fmt::Debug for GitHubUpstreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitHubUpstreamError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for GitHubUpstreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GitHub upstream response is invalid or unavailable")
    }
}

impl Error for GitHubUpstreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|value| value.as_ref() as _)
    }
}

impl GitHubService {
    #[must_use]
    pub fn http() -> Self {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("GitHub HTTP client should build");
        Self {
            inner: Arc::new(GitHubServiceInner::Http(HttpGitHubService {
                transport: GitHubTransport::Http(client),
                base_url: Url::parse(DEFAULT_BASE_URL).expect("GitHub base URL should parse"),
                clock: GitHubClock::System,
            })),
        }
    }

    #[must_use]
    pub fn mock(mock: MockGitHubService) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Mock(Box::new(mock))),
        }
    }

    pub async fn get_owner(&self, owner: &str) -> Result<Owner, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.get_owner(owner).await,
            GitHubServiceInner::Mock(service) => service.get_owner(owner),
        }
    }

    pub async fn list_repositories(
        &self,
        owner: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<RepositorySummary>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => {
                service.list_repositories(owner, limit, pagination).await
            }
            GitHubServiceInner::Mock(service) => {
                service.list_repositories(owner, limit, pagination)
            }
        }
    }

    pub async fn get_repository(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<Repository, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.get_repository(owner, repository).await,
            GitHubServiceInner::Mock(service) => service.get_repository(owner, repository),
        }
    }

    pub async fn list_activity(
        &self,
        owner: &str,
        repository: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<Activity>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => {
                service
                    .list_activity(owner, repository, limit, pagination)
                    .await
            }
            GitHubServiceInner::Mock(service) => {
                service.list_activity(owner, repository, limit, pagination)
            }
        }
    }

    pub async fn list_languages(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<Vec<Language>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.list_languages(owner, repository).await,
            GitHubServiceInner::Mock(service) => service.list_languages(owner, repository),
        }
    }

    pub async fn list_tags(
        &self,
        owner: &str,
        repository: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<Tag>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => {
                service
                    .list_tags(owner, repository, limit, pagination)
                    .await
            }
            GitHubServiceInner::Mock(service) => {
                service.list_tags(owner, repository, limit, pagination)
            }
        }
    }
}

impl Default for MockGitHubService {
    fn default() -> Self {
        Self {
            owner: None,
            repos_page: ProviderPage::default(),
            repository: None,
            activity_page: ProviderPage::default(),
            languages: Vec::new(),
            tags_page: ProviderPage::default(),
            error: None,
            calls: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockGitHubService {
    #[must_use]
    pub fn demo() -> Self {
        let summary = RepositorySummary {
            id: 1296269,
            name: "Hello-World".to_owned(),
            full_name: "octocat/Hello-World".to_owned(),
            description: Some("Synthetic repository fixture".to_owned()),
            html_url: "https://github.com/octocat/Hello-World".to_owned(),
            fork: false,
        };
        Self {
            owner: Some(Owner {
                id: 583231,
                login: "octocat".to_owned(),
                account_type: "User".to_owned(),
                name: Some("The Octocat".to_owned()),
                avatar_url: "https://avatars.githubusercontent.com/u/583231".to_owned(),
                html_url: "https://github.com/octocat".to_owned(),
                company: Some("@github".to_owned()),
                blog: Some("https://github.blog".to_owned()),
                location: Some("San Francisco".to_owned()),
                bio: None,
                public_repos: 8,
                followers: 17_000,
                following: 9,
                created_at: "2011-01-25T18:44:36.000Z".to_owned(),
                updated_at: "2024-06-01T00:00:00.000Z".to_owned(),
            }),
            repos_page: ProviderPage {
                items: vec![summary.clone()],
                next: None,
                prev: None,
            },
            repository: Some(Repository {
                id: summary.id,
                name: summary.name.clone(),
                full_name: summary.full_name.clone(),
                description: summary.description.clone(),
                html_url: summary.html_url.clone(),
                fork: summary.fork,
                language: Some("Rust".to_owned()),
                stargazers_count: 42,
                forks_count: 7,
                open_issues_count: 1,
                archived: false,
                created_at: "2011-01-26T19:01:12.000Z".to_owned(),
                updated_at: "2024-06-01T00:00:00.000Z".to_owned(),
                pushed_at: Some("2024-05-31T12:00:00.000Z".to_owned()),
                default_branch: "main".to_owned(),
                license: Some("MIT".to_owned()),
                topics: vec!["example".to_owned()],
                disabled: false,
            }),
            activity_page: ProviderPage {
                items: vec![Activity {
                    id: 1,
                    actor: Some("octocat".to_owned()),
                    actor_avatar_url: Some(
                        "https://avatars.githubusercontent.com/u/583231".to_owned(),
                    ),
                    git_ref: "refs/heads/main".to_owned(),
                    timestamp: "2024-01-15T10:30:00.000Z".to_owned(),
                    activity_type: "push".to_owned(),
                }],
                next: None,
                prev: None,
            },
            languages: vec![Language {
                name: "Rust".to_owned(),
                bytes: 6_789,
            }],
            tags_page: ProviderPage {
                items: vec![Tag {
                    name: "v1.0.0".to_owned(),
                    commit: TagCommit {
                        sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                    },
                }],
                next: None,
                prev: None,
            },
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_error(mut self, error: GitHubServiceError) -> Self {
        self.error = Some(error);
        self
    }

    #[must_use]
    pub fn with_upstream_error(mut self, kind: GitHubUpstreamErrorKind) -> Self {
        self.error = Some(upstream(kind));
        self
    }

    #[must_use]
    pub fn with_owner(mut self, owner: Owner) -> Self {
        self.owner = Some(owner);
        self
    }

    #[must_use]
    pub fn with_repos_page(mut self, page: ProviderPage<RepositorySummary>) -> Self {
        self.repos_page = page;
        self
    }

    #[must_use]
    pub fn with_activity_page(mut self, page: ProviderPage<Activity>) -> Self {
        self.activity_page = page;
        self
    }

    #[must_use]
    pub fn with_tags_page(mut self, page: ProviderPage<Tag>) -> Self {
        self.tags_page = page;
        self
    }

    #[must_use]
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn calls(&self) -> Vec<MockGitHubCall> {
        self.requests
            .lock()
            .expect("mock call lock should succeed")
            .clone()
    }

    fn record(&self, call: MockGitHubCall) -> Result<(), GitHubServiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests
            .lock()
            .expect("mock call lock should succeed")
            .push(call);
        self.error.clone().map_or(Ok(()), Err)
    }

    fn get_owner(&self, owner: &str) -> Result<Owner, GitHubServiceError> {
        self.record(MockGitHubCall {
            operation: "getGitHubOwner",
            owner: owner.to_owned(),
            repository: None,
            limit: None,
            pagination: None,
        })?;
        self.owner.clone().ok_or(GitHubServiceError::NotFound)
    }

    fn list_repositories(
        &self,
        owner: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<RepositorySummary>, GitHubServiceError> {
        self.record(MockGitHubCall {
            operation: "listGitHubOwnerRepositories",
            owner: owner.to_owned(),
            repository: None,
            limit: Some(limit),
            pagination,
        })?;
        Ok(self.repos_page.clone())
    }

    fn get_repository(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<Repository, GitHubServiceError> {
        self.record(MockGitHubCall {
            operation: "getGitHubRepository",
            owner: owner.to_owned(),
            repository: Some(repository.to_owned()),
            limit: None,
            pagination: None,
        })?;
        self.repository.clone().ok_or(GitHubServiceError::NotFound)
    }

    fn list_activity(
        &self,
        owner: &str,
        repository: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<Activity>, GitHubServiceError> {
        self.record(MockGitHubCall {
            operation: "listGitHubRepositoryActivity",
            owner: owner.to_owned(),
            repository: Some(repository.to_owned()),
            limit: Some(limit),
            pagination,
        })?;
        Ok(self.activity_page.clone())
    }

    fn list_languages(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<Vec<Language>, GitHubServiceError> {
        self.record(MockGitHubCall {
            operation: "listGitHubRepositoryLanguages",
            owner: owner.to_owned(),
            repository: Some(repository.to_owned()),
            limit: None,
            pagination: None,
        })?;
        Ok(self.languages.clone())
    }

    fn list_tags(
        &self,
        owner: &str,
        repository: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<Tag>, GitHubServiceError> {
        self.record(MockGitHubCall {
            operation: "listGitHubRepositoryTags",
            owner: owner.to_owned(),
            repository: Some(repository.to_owned()),
            limit: Some(limit),
            pagination,
        })?;
        Ok(self.tags_page.clone())
    }
}

#[derive(Clone, Debug)]
struct OperationRequest {
    url: Url,
    named_path: String,
    numeric_suffix: &'static str,
    request_query: BTreeMap<String, String>,
    navigation: NavigationKind,
}

#[derive(Clone, Debug)]
enum NavigationKind {
    None,
    Numbered {
        current: u64,
    },
    Activity {
        current: Option<(CursorDirection, String)>,
    },
}

#[derive(Debug)]
struct SuccessResponse {
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl HttpGitHubService {
    async fn with_deadline<T>(
        &self,
        future: impl Future<Output = Result<T, GitHubServiceError>>,
    ) -> Result<T, GitHubServiceError> {
        tokio::time::timeout(GITHUB_DEADLINE, future)
            .await
            .map_err(|_| GitHubServiceError::Timeout)?
    }

    async fn get_owner(&self, owner: &str) -> Result<Owner, GitHubServiceError> {
        self.with_deadline(async {
            let request =
                self.operation_request(&["users", owner], "", Vec::new(), NavigationKind::None)?;
            let response = self.send_terminal(request).await?;
            project_owner(decode_provider_json(&response.body)?)
        })
        .await
    }

    async fn list_repositories(
        &self,
        owner: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<RepositorySummary>, GitHubServiceError> {
        self.with_deadline(async {
            let page = numbered_page(pagination.as_ref())?;
            let mut query = vec![
                ("type", "owner".to_owned()),
                ("sort", "full_name".to_owned()),
                ("direction", "asc".to_owned()),
                ("per_page", limit.to_string()),
            ];
            if page != 1 {
                query.push(("page", page.to_string()));
            }
            let request = self.operation_request(
                &["users", owner, "repos"],
                "/repos",
                query,
                NavigationKind::Numbered { current: page },
            )?;
            let response = self.send_terminal(request.clone()).await?;
            let raw: Vec<RawRepositorySummary> = decode_provider_json(&response.body)?;
            if raw.len() > usize::from(limit) {
                return Err(upstream(GitHubUpstreamErrorKind::Schema));
            }
            let items = raw
                .into_iter()
                .map(project_repository_summary)
                .collect::<Result<Vec<_>, _>>()?;
            let (next, prev) = self.provider_navigation(&response, &request, items.is_empty())?;
            Ok(ProviderPage { items, next, prev })
        })
        .await
    }

    async fn get_repository(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<Repository, GitHubServiceError> {
        self.with_deadline(async {
            if !valid_github_repository(repository) {
                return Err(upstream(GitHubUpstreamErrorKind::Schema));
            }
            let request = self.operation_request(
                &["repos", owner, repository],
                "",
                Vec::new(),
                NavigationKind::None,
            )?;
            let response = self.send_terminal(request).await?;
            project_repository(decode_provider_json(&response.body)?)
        })
        .await
    }

    async fn list_activity(
        &self,
        owner: &str,
        repository: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<Activity>, GitHubServiceError> {
        self.with_deadline(async {
            if !valid_github_repository(repository) {
                return Err(upstream(GitHubUpstreamErrorKind::Schema));
            }
            let current = activity_state(pagination)?;
            let mut query = vec![
                ("direction", "desc".to_owned()),
                ("per_page", limit.to_string()),
            ];
            if let Some((direction, value)) = &current {
                query.push((
                    match direction {
                        CursorDirection::Next => "after",
                        CursorDirection::Prev => "before",
                    },
                    value.clone(),
                ));
            }
            let request = self.operation_request(
                &["repos", owner, repository, "activity"],
                "/activity",
                query,
                NavigationKind::Activity { current },
            )?;
            let response = self.send_terminal(request.clone()).await?;
            let raw: Vec<RawActivity> = decode_provider_json(&response.body)?;
            if raw.len() > usize::from(limit) {
                return Err(upstream(GitHubUpstreamErrorKind::Schema));
            }
            let items = raw
                .into_iter()
                .map(project_activity)
                .collect::<Result<Vec<_>, _>>()?;
            let (next, prev) = self.provider_navigation(&response, &request, items.is_empty())?;
            Ok(ProviderPage { items, next, prev })
        })
        .await
    }

    async fn list_languages(
        &self,
        owner: &str,
        repository: &str,
    ) -> Result<Vec<Language>, GitHubServiceError> {
        self.with_deadline(async {
            if !valid_github_repository(repository) {
                return Err(upstream(GitHubUpstreamErrorKind::Schema));
            }
            let request = self.operation_request(
                &["repos", owner, repository, "languages"],
                "/languages",
                Vec::new(),
                NavigationKind::None,
            )?;
            let response = self.send_terminal(request).await?;
            let raw: BTreeMap<String, u64> = decode_provider_json(&response.body)?;
            let mut languages = raw
                .into_iter()
                .map(|(name, bytes)| {
                    if name.is_empty() || bytes > SAFE_INTEGER_MAX {
                        Err(upstream(GitHubUpstreamErrorKind::Schema))
                    } else {
                        Ok(Language { name, bytes })
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            languages.sort_by(|left, right| {
                right
                    .bytes
                    .cmp(&left.bytes)
                    .then_with(|| left.name.cmp(&right.name))
            });
            Ok(languages)
        })
        .await
    }

    async fn list_tags(
        &self,
        owner: &str,
        repository: &str,
        limit: u16,
        pagination: Option<GitHubPagination>,
    ) -> Result<ProviderPage<Tag>, GitHubServiceError> {
        self.with_deadline(async {
            if !valid_github_repository(repository) {
                return Err(upstream(GitHubUpstreamErrorKind::Schema));
            }
            let page = numbered_page(pagination.as_ref())?;
            let mut query = vec![("per_page", limit.to_string())];
            if page != 1 {
                query.push(("page", page.to_string()));
            }
            let request = self.operation_request(
                &["repos", owner, repository, "tags"],
                "/tags",
                query,
                NavigationKind::Numbered { current: page },
            )?;
            let response = self.send_terminal(request.clone()).await?;
            let raw: Vec<RawTag> = decode_provider_json(&response.body)?;
            if raw.len() > usize::from(limit) {
                return Err(upstream(GitHubUpstreamErrorKind::Schema));
            }
            let items = raw
                .into_iter()
                .map(project_tag)
                .collect::<Result<Vec<_>, _>>()?;
            let (next, prev) = self.provider_navigation(&response, &request, items.is_empty())?;
            Ok(ProviderPage { items, next, prev })
        })
        .await
    }

    fn operation_request(
        &self,
        segments: &[&str],
        numeric_suffix: &'static str,
        query: Vec<(&str, String)>,
        navigation: NavigationKind,
    ) -> Result<OperationRequest, GitHubServiceError> {
        let mut url = self.base_url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| upstream(GitHubUpstreamErrorKind::Schema))?;
            path.clear();
            path.extend(segments.iter().copied());
        }
        {
            let mut pairs = url.query_pairs_mut();
            pairs.clear();
            for (name, value) in &query {
                pairs.append_pair(name, value);
            }
        }
        if query.is_empty() {
            url.set_query(None);
        }
        let request_query = query
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect();
        Ok(OperationRequest {
            named_path: url.path().to_owned(),
            url,
            numeric_suffix,
            request_query,
            navigation,
        })
    }

    async fn send_terminal(
        &self,
        request: OperationRequest,
    ) -> Result<SuccessResponse, GitHubServiceError> {
        let mut url = request.url.clone();
        let mut visited = BTreeSet::from([url.as_str().to_owned()]);
        let mut redirects = 0;
        loop {
            let response = self
                .transport
                .execute(outbound_request(url.clone()))
                .await?;
            validate_content_encoding(&response.headers)?;
            if is_redirect(response.status) {
                if redirects == MAX_REDIRECTS {
                    return Err(upstream(GitHubUpstreamErrorKind::Redirect));
                }
                let next = validate_redirect(&response.headers, &url, &self.base_url, &request)?;
                if !visited.insert(next.as_str().to_owned()) {
                    return Err(upstream(GitHubUpstreamErrorKind::Redirect));
                }
                redirects += 1;
                url = next;
                continue;
            }

            match response.status {
                StatusCode::OK => {
                    validate_json_content_type(&response.headers)?;
                    let headers = response.headers.clone();
                    let body = response.read_success_body().await?;
                    return Ok(SuccessResponse { url, headers, body });
                }
                StatusCode::NOT_FOUND => return Err(GitHubServiceError::NotFound),
                StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
                    return Err(GitHubServiceError::RateLimited(rate_limit_fields(
                        &response.headers,
                        self.clock,
                    )));
                }
                _ => return Err(upstream(GitHubUpstreamErrorKind::Status)),
            }
        }
    }

    fn provider_navigation(
        &self,
        response: &SuccessResponse,
        request: &OperationRequest,
        empty_page: bool,
    ) -> Result<(Option<String>, Option<String>), GitHubServiceError> {
        parse_provider_links(
            &response.headers,
            &response.url,
            &self.base_url,
            request,
            empty_page,
        )
    }
}

fn numbered_page(pagination: Option<&GitHubPagination>) -> Result<u64, GitHubServiceError> {
    match pagination {
        None => Ok(1),
        Some(GitHubPagination::Numbered(page)) if (1..=SAFE_INTEGER_MAX).contains(page) => {
            Ok(*page)
        }
        _ => Err(upstream(GitHubUpstreamErrorKind::Pagination)),
    }
}

fn activity_state(
    pagination: Option<GitHubPagination>,
) -> Result<Option<(CursorDirection, String)>, GitHubServiceError> {
    match pagination {
        None => Ok(None),
        Some(GitHubPagination::Activity { direction, value }) if valid_activity_value(&value) => {
            Ok(Some((direction, value)))
        }
        _ => Err(upstream(GitHubUpstreamErrorKind::Pagination)),
    }
}

#[derive(Debug)]
struct OutboundRequest {
    url: Url,
    headers: HeaderMap,
}

fn outbound_request(url: Url) -> OutboundRequest {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static(GITHUB_ACCEPT));
    headers.insert(
        HeaderNameExt::github_api_version(),
        HeaderValue::from_static(GITHUB_API_VERSION),
    );
    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_static(DEFAULT_USER_AGENT),
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    OutboundRequest { url, headers }
}

struct HeaderNameExt;

impl HeaderNameExt {
    fn github_api_version() -> header::HeaderName {
        header::HeaderName::from_static("x-github-api-version")
    }
}

#[derive(Debug)]
struct ProviderResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: ProviderBody,
}

#[derive(Debug)]
enum ProviderBody {
    Http(reqwest::Response),
    #[cfg(test)]
    Mock(MockGitHubBody),
}

impl GitHubTransport {
    async fn execute(
        &self,
        request: OutboundRequest,
    ) -> Result<ProviderResponse, GitHubServiceError> {
        match self {
            Self::Http(client) => {
                let response = client
                    .get(request.url)
                    .headers(request.headers)
                    .send()
                    .await
                    .map_err(|error| {
                        GitHubServiceError::Upstream(GitHubUpstreamError::with_source(
                            GitHubUpstreamErrorKind::Transport,
                            error,
                        ))
                    })?;
                Ok(ProviderResponse {
                    status: response.status(),
                    headers: response.headers().clone(),
                    body: ProviderBody::Http(response),
                })
            }
            #[cfg(test)]
            Self::Mock(transport) => transport.execute(request).await,
        }
    }
}

impl ProviderResponse {
    async fn read_success_body(self) -> Result<Vec<u8>, GitHubServiceError> {
        validate_content_length(&self.headers)?;
        match self.body {
            ProviderBody::Http(response) => {
                let mut body = Vec::new();
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|error| {
                        GitHubServiceError::Upstream(GitHubUpstreamError::with_source(
                            GitHubUpstreamErrorKind::Transport,
                            error,
                        ))
                    })?;
                    checked_response_body_len(body.len(), chunk.len())?;
                    body.extend_from_slice(&chunk);
                }
                Ok(body)
            }
            #[cfg(test)]
            ProviderBody::Mock(body) => body.read(),
        }
    }
}

fn checked_response_body_len(
    current: usize,
    additional: usize,
) -> Result<usize, GitHubServiceError> {
    current
        .checked_add(additional)
        .filter(|length| *length <= MAX_GITHUB_RESPONSE_BODY_BYTES)
        .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Size))
}

fn validate_content_length(headers: &HeaderMap) -> Result<(), GitHubServiceError> {
    let values = headers
        .get_all(header::CONTENT_LENGTH)
        .iter()
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let length = single_canonical_integer(&values, u64::MAX)
        .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Size))?;
    if length > MAX_GITHUB_RESPONSE_BODY_BYTES as u64 {
        Err(upstream(GitHubUpstreamErrorKind::Size))
    } else {
        Ok(())
    }
}

fn validate_content_encoding(headers: &HeaderMap) -> Result<(), GitHubServiceError> {
    let values = headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .collect::<Vec<_>>();
    if values.is_empty()
        || (values.len() == 1
            && values[0]
                .to_str()
                .is_ok_and(|value| !value.contains(',') && value.eq_ignore_ascii_case("identity")))
    {
        Ok(())
    } else {
        Err(upstream(GitHubUpstreamErrorKind::Encoding))
    }
}

fn validate_json_content_type(headers: &HeaderMap) -> Result<(), GitHubServiceError> {
    let values = headers
        .get_all(header::CONTENT_TYPE)
        .iter()
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(upstream(GitHubUpstreamErrorKind::Media));
    }
    let value = values[0]
        .to_str()
        .map_err(|_| upstream(GitHubUpstreamErrorKind::Media))?;
    let parts =
        split_outside_quotes(value, ';').ok_or_else(|| upstream(GitHubUpstreamErrorKind::Media))?;
    let media = parts
        .first()
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let Some((kind, subtype)) = media.split_once('/') else {
        return Err(upstream(GitHubUpstreamErrorKind::Media));
    };
    if kind != "application"
        || !valid_token(kind)
        || !valid_token(subtype)
        || !(subtype == "json" || subtype.ends_with("+json"))
    {
        return Err(upstream(GitHubUpstreamErrorKind::Media));
    }
    let mut names = BTreeSet::new();
    for parameter in parts.iter().skip(1) {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            return Err(upstream(GitHubUpstreamErrorKind::Media));
        };
        let name = name.trim().to_ascii_lowercase();
        if !valid_token(&name) || !names.insert(name) || decode_parameter(value.trim()).is_none() {
            return Err(upstream(GitHubUpstreamErrorKind::Media));
        }
    }
    Ok(())
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn validate_redirect(
    headers: &HeaderMap,
    current: &Url,
    base: &Url,
    request: &OperationRequest,
) -> Result<Url, GitHubServiceError> {
    let values = headers.get_all(header::LOCATION).iter().collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(upstream(GitHubUpstreamErrorKind::Redirect));
    }
    let location = values[0]
        .to_str()
        .map_err(|_| upstream(GitHubUpstreamErrorKind::Redirect))?;
    if location.is_empty() || location.contains(',') {
        return Err(upstream(GitHubUpstreamErrorKind::Redirect));
    }
    let target = current
        .join(location)
        .map_err(|_| upstream(GitHubUpstreamErrorKind::Redirect))?;
    validate_common_target(&target, base, request, GitHubUpstreamErrorKind::Redirect)?;
    if current.path() != request.named_path && target.path() != current.path() {
        return Err(upstream(GitHubUpstreamErrorKind::Redirect));
    }
    if strict_query(target.query())? != request.request_query {
        return Err(upstream(GitHubUpstreamErrorKind::Redirect));
    }
    Ok(target)
}

fn validate_common_target(
    target: &Url,
    base: &Url,
    request: &OperationRequest,
    kind: GitHubUpstreamErrorKind,
) -> Result<(), GitHubServiceError> {
    if !same_origin(target, base)
        || !target.username().is_empty()
        || target.password().is_some()
        || target.fragment().is_some()
        || !allowed_operation_path(target.path(), &request.named_path, request.numeric_suffix)
    {
        return Err(upstream(kind));
    }
    Ok(())
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn allowed_operation_path(path: &str, named_path: &str, numeric_suffix: &str) -> bool {
    if path == named_path {
        return true;
    }
    let numeric_prefix = if named_path.starts_with("/users/") {
        "/user/"
    } else {
        "/repositories/"
    };
    let Some(id) = path.strip_prefix(numeric_prefix) else {
        return false;
    };
    let id = if numeric_suffix.is_empty() {
        id
    } else {
        let Some(id) = id.strip_suffix(numeric_suffix) else {
            return false;
        };
        id
    };
    canonical_integer(id, SAFE_INTEGER_MAX).is_some()
}

fn parse_provider_links(
    headers: &HeaderMap,
    current_url: &Url,
    base: &Url,
    request: &OperationRequest,
    empty_page: bool,
) -> Result<(Option<String>, Option<String>), GitHubServiceError> {
    let mut relevant: BTreeMap<&'static str, String> = BTreeMap::new();
    for field in headers.get_all(header::LINK) {
        let field = field
            .to_str()
            .map_err(|_| upstream(GitHubUpstreamErrorKind::Pagination))?;
        for raw in split_link_values(field)? {
            let link = parse_link_value(raw)?;
            if link.parameters.contains_key("anchor") {
                continue;
            }
            let relations = link
                .parameters
                .get("rel")
                .map(|value| value.split_ascii_whitespace().collect::<Vec<_>>())
                .unwrap_or_default();
            for relation in ["next", "prev"] {
                let occurrences = relations.iter().filter(|value| **value == relation).count();
                if occurrences > 1 {
                    return Err(upstream(GitHubUpstreamErrorKind::Pagination));
                }
                if occurrences == 1 && relevant.insert(relation, link.target.clone()).is_some() {
                    return Err(upstream(GitHubUpstreamErrorKind::Pagination));
                }
            }
        }
    }

    let next = relevant
        .get("next")
        .map(|target| {
            validate_provider_link(target, "next", current_url, base, request, empty_page)
        })
        .transpose()?;
    let prev = relevant
        .get("prev")
        .map(|target| {
            validate_provider_link(target, "prev", current_url, base, request, empty_page)
        })
        .transpose()?;
    Ok((next, prev))
}

fn validate_provider_link(
    target: &str,
    relation: &str,
    current_url: &Url,
    base: &Url,
    request: &OperationRequest,
    empty_page: bool,
) -> Result<String, GitHubServiceError> {
    let target = current_url
        .join(target)
        .map_err(|_| upstream(GitHubUpstreamErrorKind::Pagination))?;
    validate_common_target(&target, base, request, GitHubUpstreamErrorKind::Pagination)?;
    if current_url.path() != request.named_path && target.path() != current_url.path() {
        return Err(upstream(GitHubUpstreamErrorKind::Pagination));
    }
    let query = strict_query(target.query())?;
    match &request.navigation {
        NavigationKind::None => Err(upstream(GitHubUpstreamErrorKind::Pagination)),
        NavigationKind::Numbered { current } => {
            let page = exact_navigation_query(&query, &request.request_query, &["page"])?
                .and_then(|(_, value)| canonical_integer(value, SAFE_INTEGER_MAX))
                .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?;
            if page == 0
                || (relation == "next" && (empty_page || page <= *current || page == 1))
                || (relation == "prev" && page >= *current)
            {
                return Err(upstream(GitHubUpstreamErrorKind::Pagination));
            }
            Ok(page.to_string())
        }
        NavigationKind::Activity { current } => {
            if relation == "prev" && current.is_none() {
                return Err(upstream(GitHubUpstreamErrorKind::Pagination));
            }
            let expected = if relation == "next" {
                "after"
            } else {
                "before"
            };
            let (name, value) =
                exact_navigation_query(&query, &request.request_query, &["after", "before"])?
                    .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?;
            if name != expected
                || !valid_activity_value(value)
                || (relation == "next" && empty_page)
            {
                return Err(upstream(GitHubUpstreamErrorKind::Pagination));
            }
            if current.as_ref().is_some_and(|(direction, current_value)| {
                *direction
                    == if name == "after" {
                        CursorDirection::Next
                    } else {
                        CursorDirection::Prev
                    }
                    && current_value == value
            }) {
                return Err(upstream(GitHubUpstreamErrorKind::Pagination));
            }
            Ok(value.to_owned())
        }
    }
}

fn exact_navigation_query<'a>(
    target: &'a BTreeMap<String, String>,
    request: &BTreeMap<String, String>,
    pagination_names: &[&str],
) -> Result<Option<(&'a str, &'a str)>, GitHubServiceError> {
    let fixed = request
        .iter()
        .filter(|(name, _)| !pagination_names.contains(&name.as_str()))
        .collect::<BTreeMap<_, _>>();
    for (name, value) in &fixed {
        if target.get(name.as_str()) != Some(*value) {
            return Err(upstream(GitHubUpstreamErrorKind::Pagination));
        }
    }
    let extras = target
        .iter()
        .filter(|(name, _)| !fixed.contains_key(name))
        .collect::<Vec<_>>();
    if extras.len() != 1 || !pagination_names.contains(&extras[0].0.as_str()) {
        return Err(upstream(GitHubUpstreamErrorKind::Pagination));
    }
    Ok(Some((extras[0].0.as_str(), extras[0].1.as_str())))
}

#[derive(Debug)]
struct ParsedLink {
    target: String,
    parameters: BTreeMap<String, String>,
}

fn split_link_values(value: &str) -> Result<Vec<&str>, GitHubServiceError> {
    let mut values = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut angle = false;
    for (index, character) in value.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
        } else if character == '<' {
            angle = true;
        } else if character == '>' {
            angle = false;
        } else if character == ',' && !angle {
            values.push(value[start..index].trim());
            start = index + 1;
        }
    }
    if quoted || angle {
        return Err(upstream(GitHubUpstreamErrorKind::Pagination));
    }
    values.push(value[start..].trim());
    if values.iter().any(|value| value.is_empty()) {
        Err(upstream(GitHubUpstreamErrorKind::Pagination))
    } else {
        Ok(values)
    }
}

fn parse_link_value(value: &str) -> Result<ParsedLink, GitHubServiceError> {
    let Some(end) = value.find('>') else {
        return Err(upstream(GitHubUpstreamErrorKind::Pagination));
    };
    let target = value
        .strip_prefix('<')
        .and_then(|value| value.get(..end - 1))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?;
    let rest = &value[end + 1..];
    let mut parameters = BTreeMap::new();
    if !rest.trim().is_empty() {
        for parameter in split_outside_quotes(rest, ';')
            .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?
            .into_iter()
            .filter(|value| !value.trim().is_empty())
        {
            let Some((name, raw_value)) = parameter.trim().split_once('=') else {
                return Err(upstream(GitHubUpstreamErrorKind::Pagination));
            };
            let name = name.trim().to_ascii_lowercase();
            let value = decode_parameter(raw_value.trim())
                .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?;
            if !valid_token(&name) || parameters.insert(name, value).is_some() {
                return Err(upstream(GitHubUpstreamErrorKind::Pagination));
            }
        }
    }
    Ok(ParsedLink {
        target: target.to_owned(),
        parameters,
    })
}

fn strict_query(raw: Option<&str>) -> Result<BTreeMap<String, String>, GitHubServiceError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    if raw.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut query = BTreeMap::new();
    for pair in raw.split('&') {
        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?;
        let name = decode_query_component(name)
            .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?;
        let value = decode_query_component(value)
            .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Pagination))?;
        if name.is_empty() || query.insert(name, value).is_some() {
            return Err(upstream(GitHubUpstreamErrorKind::Pagination));
        }
    }
    Ok(query)
}

fn valid_activity_value(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 2_048
        && value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
}

fn canonical_integer(value: &str, maximum: u64) -> Option<u64> {
    if value == "0" {
        return Some(0);
    }
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse::<u64>().ok().filter(|value| *value <= maximum)
}

fn single_canonical_integer(values: &[&HeaderValue], maximum: u64) -> Option<u64> {
    (values.len() == 1)
        .then(|| values[0].to_str().ok())
        .flatten()
        .filter(|value| !value.contains(','))
        .and_then(|value| canonical_integer(value, maximum))
}

fn rate_limit_fields(headers: &HeaderMap, clock: GitHubClock) -> GitHubRateLimit {
    let retry_values = headers
        .get_all(header::RETRY_AFTER)
        .iter()
        .collect::<Vec<_>>();
    let reset_values = headers
        .get_all(header::HeaderName::from_static("x-ratelimit-reset"))
        .iter()
        .collect::<Vec<_>>();
    let retry = single_canonical_integer(&retry_values, SAFE_INTEGER_MAX);
    let reset = single_canonical_integer(&reset_values, SAFE_INTEGER_MAX);
    let (now, _) = clock.now();
    let usable_reset = reset.filter(|reset| *reset > now);
    let retry_after = retry
        .or_else(|| usable_reset.map(|reset| reset - now))
        .unwrap_or(60)
        .to_string();
    GitHubRateLimit {
        retry_after,
        rate_limit_reset: usable_reset.map(|value| value.to_string()),
    }
}

impl GitHubClock {
    fn now(self) -> (u64, u32) {
        match self {
            Self::System => {
                let now = OffsetDateTime::now_utc();
                (
                    u64::try_from(now.unix_timestamp()).unwrap_or(0),
                    now.nanosecond(),
                )
            }
            #[cfg(test)]
            Self::Fixed { seconds, nanos } => (seconds, nanos),
        }
    }
}

fn decode_provider_json<T: DeserializeOwned>(body: &[u8]) -> Result<T, GitHubServiceError> {
    let value = parse_strict_json(body).map_err(|_| upstream(GitHubUpstreamErrorKind::Json))?;
    serde_json::from_value(value).map_err(|_| upstream(GitHubUpstreamErrorKind::Schema))
}

#[derive(Debug)]
struct RequiredNullable<T>(Option<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for RequiredNullable<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(Self)
    }
}

#[derive(Debug, Deserialize)]
struct RawOwner {
    id: u64,
    login: String,
    #[serde(rename = "type")]
    account_type: String,
    name: Option<String>,
    avatar_url: String,
    html_url: String,
    company: Option<String>,
    blog: Option<String>,
    location: Option<String>,
    bio: Option<String>,
    public_repos: u64,
    followers: u64,
    following: u64,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct RawRepositorySummary {
    id: u64,
    name: String,
    full_name: String,
    description: RequiredNullable<String>,
    html_url: String,
    fork: bool,
    private: bool,
    visibility: String,
}

#[derive(Debug, Deserialize)]
struct RawRepository {
    id: u64,
    name: String,
    full_name: String,
    description: RequiredNullable<String>,
    html_url: String,
    fork: bool,
    private: bool,
    visibility: String,
    language: RequiredNullable<String>,
    stargazers_count: u64,
    forks_count: u64,
    open_issues_count: u64,
    archived: bool,
    created_at: String,
    updated_at: String,
    pushed_at: RequiredNullable<String>,
    default_branch: String,
    license: Option<RawLicense>,
    #[serde(default)]
    topics: Vec<String>,
    disabled: bool,
}

#[derive(Debug, Deserialize)]
struct RawLicense {
    spdx_id: RequiredNullable<String>,
}

#[derive(Debug, Deserialize)]
struct RawActivity {
    id: u64,
    actor: RequiredNullable<RawActor>,
    #[serde(rename = "ref")]
    git_ref: String,
    timestamp: String,
    activity_type: String,
}

#[derive(Debug, Deserialize)]
struct RawActor {
    login: String,
    avatar_url: String,
}

#[derive(Debug, Deserialize)]
struct RawTag {
    name: String,
    commit: RawTagCommit,
}

#[derive(Debug, Deserialize)]
struct RawTagCommit {
    sha: String,
}

fn project_owner(raw: RawOwner) -> Result<Owner, GitHubServiceError> {
    require_safe(raw.id)?;
    require_nonempty(&raw.login)?;
    require_nonempty(&raw.account_type)?;
    require_http_url(&raw.avatar_url)?;
    require_http_url(&raw.html_url)?;
    for value in [raw.public_repos, raw.followers, raw.following] {
        require_safe(value)?;
    }
    Ok(Owner {
        id: raw.id,
        login: raw.login,
        account_type: raw.account_type,
        name: optional_display(raw.name),
        avatar_url: raw.avatar_url,
        html_url: raw.html_url,
        company: optional_display(raw.company),
        blog: optional_display(raw.blog),
        location: optional_display(raw.location),
        bio: optional_display(raw.bio),
        public_repos: raw.public_repos,
        followers: raw.followers,
        following: raw.following,
        created_at: require_timestamp(&raw.created_at)?,
        updated_at: require_timestamp(&raw.updated_at)?,
    })
}

fn project_repository_summary(
    raw: RawRepositorySummary,
) -> Result<RepositorySummary, GitHubServiceError> {
    require_public_repo(raw.private, &raw.visibility)?;
    require_safe(raw.id)?;
    require_nonempty(&raw.name)?;
    require_nonempty(&raw.full_name)?;
    require_http_url(&raw.html_url)?;
    Ok(RepositorySummary {
        id: raw.id,
        name: raw.name,
        full_name: raw.full_name,
        description: optional_display(raw.description.0),
        html_url: raw.html_url,
        fork: raw.fork,
    })
}

fn project_repository(raw: RawRepository) -> Result<Repository, GitHubServiceError> {
    require_public_repo(raw.private, &raw.visibility)?;
    for value in [
        raw.id,
        raw.stargazers_count,
        raw.forks_count,
        raw.open_issues_count,
    ] {
        require_safe(value)?;
    }
    for value in [&raw.name, &raw.full_name, &raw.default_branch] {
        require_nonempty(value)?;
    }
    require_http_url(&raw.html_url)?;
    if raw.language.0.as_deref().is_some_and(str::is_empty) {
        return Err(upstream(GitHubUpstreamErrorKind::Schema));
    }
    let mut seen = BTreeSet::new();
    if raw.topics.iter().any(|value| !seen.insert(value.clone())) {
        return Err(upstream(GitHubUpstreamErrorKind::Schema));
    }
    let mut topics = raw.topics;
    topics.sort();
    let license = raw
        .license
        .and_then(|license| license.spdx_id.0)
        .filter(|value| !value.is_empty() && value != "NOASSERTION");
    Ok(Repository {
        id: raw.id,
        name: raw.name,
        full_name: raw.full_name,
        description: optional_display(raw.description.0),
        html_url: raw.html_url,
        fork: raw.fork,
        language: raw.language.0,
        stargazers_count: raw.stargazers_count,
        forks_count: raw.forks_count,
        open_issues_count: raw.open_issues_count,
        archived: raw.archived,
        created_at: require_timestamp(&raw.created_at)?,
        updated_at: require_timestamp(&raw.updated_at)?,
        pushed_at: raw
            .pushed_at
            .0
            .map(|value| require_timestamp(&value))
            .transpose()?,
        default_branch: raw.default_branch,
        license,
        topics,
        disabled: raw.disabled,
    })
}

fn project_activity(raw: RawActivity) -> Result<Activity, GitHubServiceError> {
    require_safe(raw.id)?;
    require_nonempty(&raw.git_ref)?;
    require_nonempty(&raw.activity_type)?;
    let (actor, actor_avatar_url) = match raw.actor.0 {
        None => (None, None),
        Some(actor) => {
            require_nonempty(&actor.login)?;
            require_http_url(&actor.avatar_url)?;
            (Some(actor.login), Some(actor.avatar_url))
        }
    };
    Ok(Activity {
        id: raw.id,
        actor,
        actor_avatar_url,
        git_ref: raw.git_ref,
        timestamp: require_timestamp(&raw.timestamp)?,
        activity_type: raw.activity_type,
    })
}

fn project_tag(raw: RawTag) -> Result<Tag, GitHubServiceError> {
    require_nonempty(&raw.name)?;
    if !matches!(raw.commit.sha.len(), 40 | 64)
        || !raw
            .commit
            .sha
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(upstream(GitHubUpstreamErrorKind::Schema));
    }
    Ok(Tag {
        name: raw.name,
        commit: TagCommit {
            sha: raw.commit.sha,
        },
    })
}

fn require_public_repo(private: bool, visibility: &str) -> Result<(), GitHubServiceError> {
    if !private && visibility == "public" {
        Ok(())
    } else {
        Err(upstream(GitHubUpstreamErrorKind::Schema))
    }
}

fn require_safe(value: u64) -> Result<(), GitHubServiceError> {
    if value <= SAFE_INTEGER_MAX {
        Ok(())
    } else {
        Err(upstream(GitHubUpstreamErrorKind::Schema))
    }
}

fn require_nonempty(value: &str) -> Result<(), GitHubServiceError> {
    if value.is_empty() {
        Err(upstream(GitHubUpstreamErrorKind::Schema))
    } else {
        Ok(())
    }
}

fn require_http_url(value: &str) -> Result<(), GitHubServiceError> {
    let has_syntax_violation = Cell::new(false);
    let record_syntax_violation = |_| has_syntax_violation.set(true);
    let url = Url::options()
        .syntax_violation_callback(Some(&record_syntax_violation))
        .parse(value)
        .map_err(|_| upstream(GitHubUpstreamErrorKind::Schema))?;
    if value.is_ascii()
        && !has_syntax_violation.get()
        && matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
    {
        Ok(())
    } else {
        Err(upstream(GitHubUpstreamErrorKind::Schema))
    }
}

fn require_timestamp(value: &str) -> Result<String, GitHubServiceError> {
    normalize_timestamp(value).ok_or_else(|| upstream(GitHubUpstreamErrorKind::Schema))
}

fn optional_display(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[must_use]
pub fn valid_github_owner(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(1..=39).contains(&bytes.len()) || !value.is_ascii() {
        return false;
    }
    if bytes.len() == 1 {
        return bytes[0].is_ascii_alphanumeric();
    }
    let [first, middle @ .., last] = bytes else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && middle
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[must_use]
pub fn valid_github_repository(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=100).contains(&bytes.len())
        && value.is_ascii()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && bytes
            .iter()
            .any(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn upstream(kind: GitHubUpstreamErrorKind) -> GitHubServiceError {
    GitHubServiceError::Upstream(GitHubUpstreamError::new(kind))
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct MockGitHubTransport {
    responses:
        Arc<Mutex<std::collections::VecDeque<Result<MockGitHubResponse, GitHubServiceError>>>>,
    requests: Arc<Mutex<Vec<RecordedGitHubRequest>>>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct MockGitHubResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
    body_reads: Arc<AtomicUsize>,
    delay: Duration,
}

#[cfg(test)]
#[derive(Debug)]
struct MockGitHubBody {
    bytes: Vec<u8>,
    reads: Arc<AtomicUsize>,
}

#[cfg(test)]
impl MockGitHubBody {
    fn read(self) -> Result<Vec<u8>, GitHubServiceError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        checked_response_body_len(0, self.bytes.len())?;
        Ok(self.bytes)
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct RecordedGitHubRequest {
    url: Url,
    headers: HeaderMap,
}

#[cfg(test)]
impl MockGitHubTransport {
    fn new(responses: Vec<MockGitHubResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().map(Ok).collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_results(responses: Vec<Result<MockGitHubResponse, GitHubServiceError>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn execute(
        &self,
        request: OutboundRequest,
    ) -> Result<ProviderResponse, GitHubServiceError> {
        self.requests
            .lock()
            .expect("request lock")
            .push(RecordedGitHubRequest {
                url: request.url,
                headers: request.headers,
            });
        let response = self
            .responses
            .lock()
            .expect("response lock")
            .pop_front()
            .ok_or_else(|| upstream(GitHubUpstreamErrorKind::Transport))??;
        if !response.delay.is_zero() {
            tokio::time::sleep(response.delay).await;
        }
        Ok(ProviderResponse {
            status: response.status,
            headers: response.headers,
            body: ProviderBody::Mock(MockGitHubBody {
                bytes: response.body,
                reads: response.body_reads,
            }),
        })
    }
}

#[cfg(test)]
impl HttpGitHubService {
    fn with_mock(transport: MockGitHubTransport) -> Self {
        Self {
            transport: GitHubTransport::Mock(transport),
            base_url: Url::parse("https://api.github.test").expect("test URL"),
            clock: GitHubClock::Fixed {
                seconds: 100,
                nanos: 250_000_000,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw_response(status: StatusCode, body: impl Into<Vec<u8>>) -> MockGitHubResponse {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        MockGitHubResponse {
            status,
            headers,
            body: body.into(),
            body_reads: Arc::new(AtomicUsize::new(0)),
            delay: Duration::ZERO,
        }
    }

    fn response(status: StatusCode, body: impl Serialize) -> MockGitHubResponse {
        raw_response(status, serde_json::to_vec(&body).expect("json"))
    }

    fn owner_json() -> serde_json::Value {
        json!({
            "id": 1,
            "login": "octocat",
            "type": "User",
            "name": null,
            "avatar_url": "https://avatars.example.test/1",
            "html_url": "https://github.example.test/octocat",
            "company": null,
            "blog": null,
            "location": null,
            "bio": null,
            "public_repos": 1,
            "followers": 2,
            "following": 3,
            "created_at": "2020-01-01T00:00:00.000Z",
            "updated_at": "2020-01-02T00:00:00.000Z"
        })
    }

    fn repository_summary_json(id: u64, name: &str) -> serde_json::Value {
        json!({
            "id": id,
            "name": name,
            "full_name": format!("octocat/{name}"),
            "description": null,
            "html_url": format!("https://github.example.test/octocat/{name}"),
            "fork": false,
            "private": false,
            "visibility": "public"
        })
    }

    fn repository_json() -> serde_json::Value {
        json!({
            "id": 1,
            "name": "repo",
            "full_name": "octocat/repo",
            "description": null,
            "html_url": "https://github.example.test/octocat/repo",
            "fork": false,
            "private": false,
            "visibility": "public",
            "language": null,
            "stargazers_count": 1,
            "forks_count": 2,
            "open_issues_count": 3,
            "archived": false,
            "created_at": "2020-01-01T00:00:00.000Z",
            "updated_at": "2020-01-02T00:00:00.000Z",
            "pushed_at": null,
            "default_branch": "main",
            "license": null,
            "topics": [],
            "disabled": false
        })
    }

    fn activity_json(id: u64) -> serde_json::Value {
        json!({
            "id": id,
            "actor": null,
            "ref": "refs/heads/main",
            "timestamp": "2020-01-02T00:00:00.000Z",
            "activity_type": "push"
        })
    }

    fn tag_json(name: &str) -> serde_json::Value {
        json!({
            "name": name,
            "commit": {"sha": "0123456789abcdef0123456789abcdef01234567"}
        })
    }

    fn redirect_response(status: StatusCode, location: Option<&str>) -> MockGitHubResponse {
        let mut response = raw_response(status, br#"{"secret":"unread"}"#.to_vec());
        response.headers.remove(header::CONTENT_TYPE);
        if let Some(location) = location {
            response.headers.insert(
                header::LOCATION,
                HeaderValue::from_str(location).expect("test location"),
            );
        }
        response
    }

    fn assert_upstream<T>(
        result: Result<T, GitHubServiceError>,
        expected: GitHubUpstreamErrorKind,
    ) {
        match result {
            Err(GitHubServiceError::Upstream(error)) => assert_eq!(error.kind, expected),
            _ => panic!("expected upstream {expected:?}"),
        }
    }

    #[test]
    fn path_grammars_cover_exact_boundaries() {
        for owner in ["a", "a_b", "a-b", &format!("a{}z", "x".repeat(37))] {
            assert!(valid_github_owner(owner));
        }
        for owner in ["", "-a", "a-", "a.b", "é", &"a".repeat(40)] {
            assert!(!valid_github_owner(owner));
        }
        for repo in ["a", "_", "-", "a.b", &"a".repeat(100)] {
            assert!(valid_github_repository(repo));
        }
        for repo in ["", ".", "..", "...", "a/b", "é", &"a".repeat(101)] {
            assert!(!valid_github_repository(repo));
        }
    }

    #[test]
    fn service_debug_and_upstream_errors_are_safe_but_keep_sources() {
        let service = HttpGitHubService::with_mock(MockGitHubTransport::new(Vec::new()));
        assert_eq!(
            format!("{service:?}"),
            "HttpGitHubService { base_origin: \"https://api.github.test\", .. }"
        );
        let error = GitHubUpstreamError::with_source(
            GitHubUpstreamErrorKind::Transport,
            std::io::Error::other("private sentinel"),
        );
        assert_eq!(
            format!("{error:?}"),
            "GitHubUpstreamError { kind: Transport, .. }"
        );
        assert_eq!(
            error.to_string(),
            "GitHub upstream response is invalid or unavailable"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("private sentinel")
        );
    }

    #[test]
    fn mock_builders_preserve_the_supplied_owner_and_tag_page() {
        let owner =
            project_owner(serde_json::from_value(owner_json()).expect("raw owner")).expect("owner");
        let tag =
            project_tag(serde_json::from_value(tag_json("v9.9.9")).expect("raw tag")).expect("tag");
        let page = ProviderPage {
            items: vec![tag],
            next: Some("2".to_owned()),
            prev: Some("1".to_owned()),
        };
        let mock = MockGitHubService::default()
            .with_owner(owner.clone())
            .with_tags_page(page.clone());
        assert_eq!(mock.get_owner("selected").expect("owner"), owner);
        assert_eq!(
            mock.list_tags("selected", "repo", 7, None).expect("tags"),
            page
        );
        assert_eq!(mock.call_count(), 2);
    }

    #[test]
    fn pagination_states_and_response_sizes_cover_exact_boundaries() {
        assert_eq!(numbered_page(None).expect("first page"), 1);
        assert_eq!(
            numbered_page(Some(&GitHubPagination::Numbered(SAFE_INTEGER_MAX))).expect("maximum"),
            SAFE_INTEGER_MAX
        );
        for pagination in [
            GitHubPagination::Numbered(0),
            GitHubPagination::Numbered(SAFE_INTEGER_MAX + 1),
            GitHubPagination::Activity {
                direction: CursorDirection::Next,
                value: "state".to_owned(),
            },
        ] {
            assert_upstream(
                numbered_page(Some(&pagination)),
                GitHubUpstreamErrorKind::Pagination,
            );
        }

        for value in ["!", &"x".repeat(2_048)] {
            assert!(
                activity_state(Some(GitHubPagination::Activity {
                    direction: CursorDirection::Prev,
                    value: value.to_owned(),
                }))
                .is_ok()
            );
        }
        for value in [
            String::new(),
            "x".repeat(2_049),
            "has space".to_owned(),
            "é".to_owned(),
        ] {
            assert_upstream(
                activity_state(Some(GitHubPagination::Activity {
                    direction: CursorDirection::Next,
                    value,
                })),
                GitHubUpstreamErrorKind::Pagination,
            );
        }

        assert_eq!(
            checked_response_body_len(MAX_GITHUB_RESPONSE_BODY_BYTES - 1, 1).expect("exact"),
            MAX_GITHUB_RESPONSE_BODY_BYTES
        );
        assert_upstream(
            checked_response_body_len(MAX_GITHUB_RESPONSE_BODY_BYTES, 1),
            GitHubUpstreamErrorKind::Size,
        );
        assert_upstream(
            checked_response_body_len(usize::MAX, 1),
            GitHubUpstreamErrorKind::Size,
        );
    }

    #[tokio::test]
    async fn activity_and_tag_pages_accept_exactly_the_requested_limit() {
        let activity = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
            StatusCode::OK,
            json!([activity_json(1)]),
        )]))
        .list_activity("octocat", "repo", 1, None)
        .await
        .expect("exact activity page");
        assert_eq!(activity.items.len(), 1);

        let tags = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
            StatusCode::OK,
            json!([tag_json("v1")]),
        )]))
        .list_tags("octocat", "repo", 1, None)
        .await
        .expect("exact tag page");
        assert_eq!(tags.items.len(), 1);
    }

    #[tokio::test]
    async fn languages_require_nonempty_names_and_safe_integer_counts() {
        let accepted = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
            StatusCode::OK,
            json!({"Rust": SAFE_INTEGER_MAX}),
        )]))
        .list_languages("octocat", "repo")
        .await
        .expect("safe maximum");
        assert_eq!(
            accepted,
            [Language {
                name: "Rust".to_owned(),
                bytes: SAFE_INTEGER_MAX
            }]
        );

        for body in [json!({"": 1}), json!({"Rust": SAFE_INTEGER_MAX + 1})] {
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
                    StatusCode::OK,
                    body,
                )]))
                .list_languages("octocat", "repo")
                .await,
                GitHubUpstreamErrorKind::Schema,
            );
        }
    }

    #[test]
    fn provider_media_encoding_and_redirect_metadata_are_strict() {
        let mut headers = HeaderMap::new();
        assert!(validate_content_encoding(&headers).is_ok());
        headers.insert(
            header::CONTENT_ENCODING,
            HeaderValue::from_static("IDENTITY"),
        );
        assert!(validate_content_encoding(&headers).is_ok());
        for value in ["gzip", "identity,gzip", "identity, identity"] {
            headers.insert(
                header::CONTENT_ENCODING,
                HeaderValue::from_str(value).expect("encoding"),
            );
            assert_upstream(
                validate_content_encoding(&headers),
                GitHubUpstreamErrorKind::Encoding,
            );
        }
        headers.append(
            header::CONTENT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        assert_upstream(
            validate_content_encoding(&headers),
            GitHubUpstreamErrorKind::Encoding,
        );

        for value in [
            "application/json",
            "Application/Vnd.GitHub+Json; charset=\"utf\\-8\"",
            "application/problem+json; profile=portable",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(value).expect("media"),
            );
            assert!(validate_json_content_type(&headers).is_ok(), "{value}");
        }
        for value in [
            "text/json",
            "application/xml",
            "application/bad space+json",
            "application/json, application/json",
            "application/json; charset=utf-8; charset=UTF-8",
            "application/json; bad name=value",
            "application/json; charset=\"unterminated",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(value).expect("media"),
            );
            assert_upstream(
                validate_json_content_type(&headers),
                GitHubUpstreamErrorKind::Media,
            );
        }
        assert_upstream(
            validate_json_content_type(&HeaderMap::new()),
            GitHubUpstreamErrorKind::Media,
        );

        let service = HttpGitHubService::with_mock(MockGitHubTransport::new(Vec::new()));
        let request = service
            .operation_request(&["users", "octocat"], "", Vec::new(), NavigationKind::None)
            .expect("request");
        for location in ["", "/users/octocat, /user/1"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::LOCATION,
                HeaderValue::from_str(location).expect("location"),
            );
            assert_upstream(
                validate_redirect(&headers, &request.url, &service.base_url, &request),
                GitHubUpstreamErrorKind::Redirect,
            );
        }
    }

    #[test]
    fn projection_helpers_reject_each_invalid_boundary() {
        assert!(require_nonempty("x").is_ok());
        assert_upstream(require_nonempty(""), GitHubUpstreamErrorKind::Schema);
        for value in [
            "http://example.test/path",
            "https://example.test",
            "https://EXAMPLE.test:443/caf%C3%A9?name=value#fragment",
        ] {
            assert!(require_http_url(value).is_ok(), "{value}");
        }
        for value in [
            "data:text/plain,x",
            "ftp://example.test/path",
            "file:///tmp/x",
            "https://",
            "not a URL",
            " https://example.test/path",
            "https://example.test/path ",
            "https://exa\tmple.test/path",
            "https://example.test/pa\nth",
            "https://example.test/pa\rth",
            "https://example.test/a b",
            "https://example.test/%zz",
            "https://token@example.test/path",
            "https://:secret@example.test/path",
            "https://example.test/café",
            "https://例え.test/path",
        ] {
            assert_upstream(require_http_url(value), GitHubUpstreamErrorKind::Schema);
        }
        assert_eq!(optional_display(None), None);
        assert_eq!(optional_display(Some(String::new())), None);
        assert_eq!(
            optional_display(Some("value".to_owned())).as_deref(),
            Some("value")
        );

        for sha in ["a".repeat(40), "0".repeat(64)] {
            let tag = RawTag {
                name: "tag".to_owned(),
                commit: RawTagCommit { sha: sha.clone() },
            };
            assert_eq!(project_tag(tag).expect("valid tag").commit.sha, sha);
        }
        for sha in [
            "a".repeat(39),
            "a".repeat(41),
            "A".repeat(40),
            "g".repeat(40),
        ] {
            assert_upstream(
                project_tag(RawTag {
                    name: "tag".to_owned(),
                    commit: RawTagCommit { sha },
                }),
                GitHubUpstreamErrorKind::Schema,
            );
        }

        for (license, expected) in [
            (None, None),
            (Some(""), None),
            (Some("NOASSERTION"), None),
            (Some("MIT"), Some("MIT")),
        ] {
            let mut value = repository_json();
            value["license"] = match license {
                None => json!(null),
                Some(value) => json!({"spdx_id": value}),
            };
            let projected = project_repository(serde_json::from_value(value).expect("raw repo"))
                .expect("repository");
            assert_eq!(projected.license.as_deref(), expected);
        }
    }

    #[test]
    fn projection_rejects_unsafe_url_text_in_every_public_shape() {
        for invalid in [
            " https://example.test/path",
            "https://example.test/pa\nth",
            "https://example.test/a b",
            "https://token@example.test/path",
            "https://example.test/café",
            "https://例え.test/path",
        ] {
            for field in ["avatar_url", "html_url"] {
                let mut value = owner_json();
                value[field] = json!(invalid);
                assert_upstream(
                    project_owner(serde_json::from_value(value).expect("raw owner")),
                    GitHubUpstreamErrorKind::Schema,
                );
            }

            let mut summary = repository_summary_json(1, "repo");
            summary["html_url"] = json!(invalid);
            assert_upstream(
                project_repository_summary(
                    serde_json::from_value(summary).expect("raw repository summary"),
                ),
                GitHubUpstreamErrorKind::Schema,
            );

            let mut repository = repository_json();
            repository["html_url"] = json!(invalid);
            assert_upstream(
                project_repository(serde_json::from_value(repository).expect("raw repository")),
                GitHubUpstreamErrorKind::Schema,
            );

            let mut activity = activity_json(1);
            activity["actor"] = json!({"login": "octocat", "avatar_url": invalid});
            assert_upstream(
                project_activity(serde_json::from_value(activity).expect("raw activity")),
                GitHubUpstreamErrorKind::Schema,
            );
        }
    }

    #[tokio::test]
    async fn outbound_request_has_only_the_four_application_headers() {
        let transport = MockGitHubTransport::new(vec![response(StatusCode::OK, owner_json())]);
        let service = HttpGitHubService::with_mock(transport.clone());
        service.get_owner("octocat").await.expect("owner");
        let requests = transport.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url.as_str(),
            "https://api.github.test/users/octocat"
        );
        let headers = &requests[0].headers;
        assert_eq!(headers.len(), 4);
        assert_eq!(
            headers.get(header::ACCEPT).unwrap(),
            "application/vnd.github+json"
        );
        assert_eq!(headers.get("x-github-api-version").unwrap(), "2026-03-10");
        assert_eq!(
            headers.get(header::USER_AGENT).unwrap(),
            "axum-playground/0.1.0"
        );
        assert_eq!(headers.get(header::ACCEPT_ENCODING).unwrap(), "identity");
        assert!(headers.get(header::AUTHORIZATION).is_none());
    }

    #[tokio::test]
    async fn paginated_operations_reconstruct_exact_provider_queries() {
        let transport = MockGitHubTransport::new(vec![
            response(StatusCode::OK, json!([])),
            response(StatusCode::OK, json!([])),
            response(StatusCode::OK, json!([])),
        ]);
        let service = HttpGitHubService::with_mock(transport.clone());
        service
            .list_repositories("octocat", 17, Some(GitHubPagination::Numbered(3)))
            .await
            .expect("repositories");
        service
            .list_activity(
                "octocat",
                "repo",
                9,
                Some(GitHubPagination::Activity {
                    direction: CursorDirection::Next,
                    value: "opaque/?".to_owned(),
                }),
            )
            .await
            .expect("activity");
        service
            .list_tags("octocat", "repo", 100, Some(GitHubPagination::Numbered(2)))
            .await
            .expect("tags");

        let requests = transport.requests.lock().expect("requests");
        assert_eq!(
            requests[0].url.as_str(),
            "https://api.github.test/users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=17&page=3"
        );
        assert_eq!(
            requests[1].url.as_str(),
            "https://api.github.test/repos/octocat/repo/activity?direction=desc&per_page=9&after=opaque%2F%3F"
        );
        assert_eq!(
            requests[2].url.as_str(),
            "https://api.github.test/repos/octocat/repo/tags?per_page=100&page=2"
        );
    }

    #[tokio::test]
    async fn transport_failure_is_not_retried() {
        let transport = MockGitHubTransport::with_results(vec![Err(upstream(
            GitHubUpstreamErrorKind::Transport,
        ))]);
        let service = HttpGitHubService::with_mock(transport.clone());
        assert_upstream(
            service.get_owner("octocat").await,
            GitHubUpstreamErrorKind::Transport,
        );
        assert_eq!(transport.requests.lock().expect("requests").len(), 1);
    }

    #[tokio::test]
    async fn every_operation_accepts_its_exact_numeric_redirect_path() {
        let cases = vec![
            (
                vec!["users", "octocat"],
                "",
                Vec::new(),
                NavigationKind::None,
                "/user/1".to_owned(),
            ),
            (
                vec!["users", "octocat", "repos"],
                "/repos",
                vec![
                    ("type", "owner".to_owned()),
                    ("sort", "full_name".to_owned()),
                    ("direction", "asc".to_owned()),
                    ("per_page", "20".to_owned()),
                ],
                NavigationKind::Numbered { current: 1 },
                "/user/1/repos?type=owner&sort=full_name&direction=asc&per_page=20".to_owned(),
            ),
            (
                vec!["repos", "octocat", "repo"],
                "",
                Vec::new(),
                NavigationKind::None,
                "/repositories/1".to_owned(),
            ),
            (
                vec!["repos", "octocat", "repo", "activity"],
                "/activity",
                vec![
                    ("direction", "desc".to_owned()),
                    ("per_page", "20".to_owned()),
                ],
                NavigationKind::Activity { current: None },
                "/repositories/1/activity?direction=desc&per_page=20".to_owned(),
            ),
            (
                vec!["repos", "octocat", "repo", "languages"],
                "/languages",
                Vec::new(),
                NavigationKind::None,
                "/repositories/1/languages".to_owned(),
            ),
            (
                vec!["repos", "octocat", "repo", "tags"],
                "/tags",
                vec![("per_page", "20".to_owned())],
                NavigationKind::Numbered { current: 1 },
                "/repositories/1/tags?per_page=20".to_owned(),
            ),
        ];

        for (segments, suffix, query, navigation, target) in cases {
            let redirect = redirect_response(StatusCode::MOVED_PERMANENTLY, Some(&target));
            let redirect_reads = Arc::clone(&redirect.body_reads);
            let transport = MockGitHubTransport::new(vec![
                redirect,
                raw_response(StatusCode::OK, b"{}".to_vec()),
            ]);
            let service = HttpGitHubService::with_mock(transport.clone());
            let request = service
                .operation_request(&segments, suffix, query, navigation)
                .expect("operation request");
            service.send_terminal(request).await.expect("redirect");
            let requests = transport.requests.lock().expect("requests");
            assert_eq!(requests.len(), 2, "{target}");
            assert_eq!(requests[1].url.path(), target.split('?').next().unwrap());
            assert_eq!(redirect_reads.load(Ordering::SeqCst), 0);
            for request in requests.iter() {
                assert_eq!(request.headers.len(), 4);
                assert_eq!(request.headers[header::ACCEPT], GITHUB_ACCEPT);
                assert_eq!(request.headers[header::USER_AGENT], "axum-playground/0.1.0");
                assert_eq!(request.headers[header::ACCEPT_ENCODING], "identity");
                assert_eq!(request.headers["x-github-api-version"], "2026-03-10");
            }
        }
    }

    #[tokio::test]
    async fn redirect_limit_loop_and_target_guards_fail_closed() {
        let query_a = "type=owner&sort=full_name&direction=asc&per_page=20";
        let query_b = "sort=full_name&type=owner&direction=asc&per_page=20";
        let query_c = "direction=asc&sort=full_name&type=owner&per_page=20";
        let query_d = "per_page=20&direction=asc&sort=full_name&type=owner";
        let three = MockGitHubTransport::new(vec![
            redirect_response(StatusCode::FOUND, Some(&format!("/user/1/repos?{query_a}"))),
            redirect_response(
                StatusCode::SEE_OTHER,
                Some(&format!("/user/1/repos?{query_b}")),
            ),
            redirect_response(
                StatusCode::TEMPORARY_REDIRECT,
                Some(&format!("/user/1/repos?{query_c}")),
            ),
            response(StatusCode::OK, json!([])),
        ]);
        HttpGitHubService::with_mock(three.clone())
            .list_repositories("octocat", 20, None)
            .await
            .expect("three same-identity redirects");
        assert_eq!(three.requests.lock().expect("requests").len(), 4);

        let four = MockGitHubTransport::new(vec![
            redirect_response(StatusCode::FOUND, Some(&format!("/user/1/repos?{query_a}"))),
            redirect_response(StatusCode::FOUND, Some(&format!("/user/1/repos?{query_b}"))),
            redirect_response(StatusCode::FOUND, Some(&format!("/user/1/repos?{query_c}"))),
            redirect_response(
                StatusCode::PERMANENT_REDIRECT,
                Some(&format!("/user/1/repos?{query_d}")),
            ),
        ]);
        assert_upstream(
            HttpGitHubService::with_mock(four.clone())
                .list_repositories("octocat", 20, None)
                .await,
            GitHubUpstreamErrorKind::Redirect,
        );
        assert_eq!(four.requests.lock().expect("requests").len(), 4);

        for (segments, first, second) in [
            (vec!["users", "octocat"], "/user/1", "/user/2"),
            (
                vec!["repos", "octocat", "hello-world"],
                "/repositories/1",
                "/repositories/2",
            ),
        ] {
            let first_response = redirect_response(StatusCode::FOUND, Some(first));
            let first_reads = Arc::clone(&first_response.body_reads);
            let second_response = redirect_response(StatusCode::FOUND, Some(second));
            let second_reads = Arc::clone(&second_response.body_reads);
            let transport = MockGitHubTransport::new(vec![first_response, second_response]);
            let service = HttpGitHubService::with_mock(transport.clone());
            let request = service
                .operation_request(&segments, "", Vec::new(), NavigationKind::None)
                .expect("request");
            assert_upstream(
                service.send_terminal(request).await,
                GitHubUpstreamErrorKind::Redirect,
            );
            assert_eq!(transport.requests.lock().expect("requests").len(), 2);
            assert_eq!(first_reads.load(Ordering::SeqCst), 0);
            assert_eq!(second_reads.load(Ordering::SeqCst), 0);
        }

        for location in [
            "/users/octocat",
            "https://other.example/user/1",
            "http://api.github.test/user/1",
            "https://name@api.github.test/user/1",
            "/user/1#fragment",
            "/repositories/1",
            "/user/01",
            "/user/1?unexpected=1",
            "/user/1,/user/2",
        ] {
            let redirect = redirect_response(StatusCode::FOUND, Some(location));
            let reads = Arc::clone(&redirect.body_reads);
            let transport = MockGitHubTransport::new(vec![redirect]);
            assert_upstream(
                HttpGitHubService::with_mock(transport.clone())
                    .get_owner("octocat")
                    .await,
                GitHubUpstreamErrorKind::Redirect,
            );
            assert_eq!(transport.requests.lock().expect("requests").len(), 1);
            assert_eq!(reads.load(Ordering::SeqCst), 0, "{location}");
        }

        let missing = MockGitHubTransport::new(vec![redirect_response(StatusCode::FOUND, None)]);
        assert_upstream(
            HttpGitHubService::with_mock(missing)
                .get_owner("octocat")
                .await,
            GitHubUpstreamErrorKind::Redirect,
        );
        let mut repeated = redirect_response(StatusCode::FOUND, Some("/user/1"));
        repeated
            .headers
            .append(header::LOCATION, HeaderValue::from_static("/user/2"));
        assert_upstream(
            HttpGitHubService::with_mock(MockGitHubTransport::new(vec![repeated]))
                .get_owner("octocat")
                .await,
            GitHubUpstreamErrorKind::Redirect,
        );
    }

    #[tokio::test]
    async fn content_encoding_precedes_redirect_and_status_mapping() {
        for status in [
            StatusCode::FOUND,
            StatusCode::NOT_FOUND,
            StatusCode::FORBIDDEN,
        ] {
            let mut encoded = if status == StatusCode::FOUND {
                redirect_response(status, Some("/user/1"))
            } else {
                response(status, json!({"secret": "unread"}))
            };
            let reads = Arc::clone(&encoded.body_reads);
            encoded
                .headers
                .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![encoded]))
                    .get_owner("octocat")
                    .await,
                GitHubUpstreamErrorKind::Encoding,
            );
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn error_and_redirect_bodies_are_never_read() {
        let mut error = response(StatusCode::NOT_FOUND, json!({"secret":"must-not-read"}));
        let reads = Arc::clone(&error.body_reads);
        error.headers.remove(header::CONTENT_TYPE);
        let service = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![error]));
        assert!(matches!(
            service.get_owner("octocat").await,
            Err(GitHubServiceError::NotFound)
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn success_media_type_is_exact_and_error_media_is_never_sniffed() {
        for content_type in [
            "application/json",
            "APPLICATION/JSON; CHARSET=utf-8",
            "application/vnd.github+json; profile=public",
            "application/problem+json; note=\"quoted value\"",
        ] {
            let mut success = response(StatusCode::OK, owner_json());
            success.headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(content_type).expect("content type"),
            );
            HttpGitHubService::with_mock(MockGitHubTransport::new(vec![success]))
                .get_owner("octocat")
                .await
                .expect(content_type);
        }

        for content_type in [
            None,
            Some("text/json"),
            Some("application/xml"),
            Some("application/json; broken"),
            Some("application/json; charset=utf-8; CHARSET=ascii"),
            Some("application/json, application/json"),
        ] {
            let mut success = response(StatusCode::OK, owner_json());
            success.headers.remove(header::CONTENT_TYPE);
            if let Some(content_type) = content_type {
                success.headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(content_type).expect("content type"),
                );
            }
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![success]))
                    .get_owner("octocat")
                    .await,
                GitHubUpstreamErrorKind::Media,
            );
        }

        let mut repeated = response(StatusCode::OK, owner_json());
        repeated.headers.append(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        assert_upstream(
            HttpGitHubService::with_mock(MockGitHubTransport::new(vec![repeated]))
                .get_owner("octocat")
                .await,
            GitHubUpstreamErrorKind::Media,
        );

        for status in [StatusCode::NOT_FOUND, StatusCode::INTERNAL_SERVER_ERROR] {
            let mut error = raw_response(status, vec![0xff; MAX_GITHUB_RESPONSE_BODY_BYTES + 1]);
            let reads = Arc::clone(&error.body_reads);
            error.headers.remove(header::CONTENT_TYPE);
            error
                .headers
                .insert(header::CONTENT_LENGTH, HeaderValue::from_static("4194305"));
            let result = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![error]))
                .get_owner("octocat")
                .await;
            if status == StatusCode::NOT_FOUND {
                assert!(matches!(result, Err(GitHubServiceError::NotFound)));
            } else {
                assert_upstream(result, GitHubUpstreamErrorKind::Status);
            }
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn success_body_bound_uses_the_decoded_stream_not_only_content_length() {
        let mut exact = serde_json::to_vec(&owner_json()).expect("owner JSON");
        exact.resize(MAX_GITHUB_RESPONSE_BODY_BYTES, b' ');
        let mut exact_response = raw_response(StatusCode::OK, exact.clone());
        exact_response
            .headers
            .insert(header::CONTENT_LENGTH, HeaderValue::from_static("4194304"));
        HttpGitHubService::with_mock(MockGitHubTransport::new(vec![exact_response]))
            .get_owner("octocat")
            .await
            .expect("exact body limit");

        let mut oversized = exact;
        oversized.push(b' ');
        for declared in [None, Some("1")] {
            let mut oversized_response = raw_response(StatusCode::OK, oversized.clone());
            if let Some(declared) = declared {
                oversized_response.headers.insert(
                    header::CONTENT_LENGTH,
                    HeaderValue::from_str(declared).unwrap(),
                );
            }
            let reads = Arc::clone(&oversized_response.body_reads);
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![oversized_response]))
                    .get_owner("octocat")
                    .await,
                GitHubUpstreamErrorKind::Size,
            );
            assert_eq!(reads.load(Ordering::SeqCst), 1);
        }

        let mut early = raw_response(StatusCode::OK, b"{}".to_vec());
        early
            .headers
            .insert(header::CONTENT_LENGTH, HeaderValue::from_static("4194305"));
        let reads = Arc::clone(&early.body_reads);
        assert_upstream(
            HttpGitHubService::with_mock(MockGitHubTransport::new(vec![early]))
                .get_owner("octocat")
                .await,
            GitHubUpstreamErrorKind::Size,
        );
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn strict_json_and_projection_fail_closed_while_ignoring_additive_fields() {
        let mut additive = owner_json();
        additive["future_provider_field"] = json!({"secret": true});
        additive["name"] = json!("");
        let owner = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
            StatusCode::OK,
            additive,
        )]))
        .get_owner("octocat")
        .await
        .expect("additive data");
        assert_eq!(owner.name, None);

        let canonical = serde_json::to_string(&owner_json()).expect("owner");
        let duplicate = format!("{{\"id\":1,{}", canonical.strip_prefix('{').unwrap());
        for body in [
            duplicate.into_bytes(),
            format!("{canonical} {{}}").into_bytes(),
            vec![0xff],
        ] {
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![raw_response(
                    StatusCode::OK,
                    body,
                )]))
                .get_owner("octocat")
                .await,
                GitHubUpstreamErrorKind::Json,
            );
        }

        let mut invalid = Vec::new();
        let mut missing = owner_json();
        missing.as_object_mut().unwrap().remove("login");
        invalid.push(missing);
        let mut wrong_type = owner_json();
        wrong_type["followers"] = json!("2");
        invalid.push(wrong_type);
        let mut unsafe_integer = owner_json();
        unsafe_integer["id"] = json!(9_007_199_254_740_992_u64);
        invalid.push(unsafe_integer);
        let mut bad_url = owner_json();
        bad_url["avatar_url"] = json!("data:text/plain,private");
        invalid.push(bad_url);
        let mut bad_time = owner_json();
        bad_time["created_at"] = json!("2020-01-01T00:00:00.0001Z");
        invalid.push(bad_time);
        for value in invalid {
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
                    StatusCode::OK,
                    value,
                )]))
                .get_owner("octocat")
                .await,
                GitHubUpstreamErrorKind::Schema,
            );
        }
    }

    #[tokio::test]
    async fn projection_preserves_page_order_and_uses_unicode_scalar_sorting() {
        let repos = json!([
            repository_summary_json(2, "z-last"),
            repository_summary_json(1, "a-first")
        ]);
        let service = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
            StatusCode::OK,
            repos,
        )]));
        let page = service
            .list_repositories("octocat", 2, None)
            .await
            .expect("repository page");
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["z-last", "a-first"]
        );

        let mut repository = repository_json();
        repository["topics"] = json!(["\u{10000}", "", "\u{e000}"]);
        let detail = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
            StatusCode::OK,
            repository,
        )]))
        .get_repository("octocat", "repo")
        .await
        .expect("repository detail");
        assert_eq!(detail.topics, ["", "\u{e000}", "\u{10000}"]);

        let languages = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response(
            StatusCode::OK,
            json!({"\u{10000}": 7, "\u{e000}": 7, "Rust": 9}),
        )]))
        .list_languages("octocat", "repo")
        .await
        .expect("languages");
        assert_eq!(
            languages
                .iter()
                .map(|language| (language.name.as_str(), language.bytes))
                .collect::<Vec<_>>(),
            [("Rust", 9), ("\u{e000}", 7), ("\u{10000}", 7)]
        );
    }

    #[tokio::test]
    async fn every_paginated_projection_rejects_an_over_limit_page() {
        let cases = [
            (
                response(
                    StatusCode::OK,
                    json!([
                        repository_summary_json(1, "one"),
                        repository_summary_json(2, "two")
                    ]),
                ),
                "repositories",
            ),
            (
                response(StatusCode::OK, json!([activity_json(1), activity_json(2)])),
                "activity",
            ),
            (
                response(StatusCode::OK, json!([tag_json("one"), tag_json("two")])),
                "tags",
            ),
        ];
        for (response, operation) in cases {
            let service = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![response]));
            let result = match operation {
                "repositories" => service
                    .list_repositories("octocat", 1, None)
                    .await
                    .map(|_| ()),
                "activity" => service
                    .list_activity("octocat", "repo", 1, None)
                    .await
                    .map(|_| ()),
                "tags" => service
                    .list_tags("octocat", "repo", 1, None)
                    .await
                    .map(|_| ()),
                _ => unreachable!(),
            };
            assert_upstream(result, GitHubUpstreamErrorKind::Schema);
        }
    }

    #[test]
    fn quota_hints_are_independent_and_use_exact_fallbacks() {
        let mut headers = HeaderMap::new();
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("invalid"));
        headers.insert("x-ratelimit-reset", HeaderValue::from_static("102"));
        assert_eq!(
            rate_limit_fields(
                &headers,
                GitHubClock::Fixed {
                    seconds: 100,
                    nanos: 250_000_000
                }
            ),
            GitHubRateLimit {
                retry_after: "2".to_owned(),
                rate_limit_reset: Some("102".to_owned())
            }
        );
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("7"));
        assert_eq!(
            rate_limit_fields(
                &headers,
                GitHubClock::Fixed {
                    seconds: 100,
                    nanos: 0
                }
            )
            .retry_after,
            "7"
        );

        let mut boundary = HeaderMap::new();
        boundary.insert("x-ratelimit-reset", HeaderValue::from_static("100"));
        let boundary = rate_limit_fields(
            &boundary,
            GitHubClock::Fixed {
                seconds: 100,
                nanos: 0,
            },
        );
        assert_eq!(boundary.retry_after, "60");
        assert_eq!(boundary.rate_limit_reset, None);

        for value in [
            "00",
            "+1",
            "1, 2",
            "9007199254740992",
            "184467440737095516160",
        ] {
            let mut invalid = HeaderMap::new();
            invalid.insert(
                header::RETRY_AFTER,
                HeaderValue::from_str(value).expect("header value"),
            );
            invalid.insert(
                "x-ratelimit-reset",
                HeaderValue::from_str(value).expect("header value"),
            );
            assert_eq!(
                rate_limit_fields(
                    &invalid,
                    GitHubClock::Fixed {
                        seconds: 100,
                        nanos: 0
                    }
                ),
                GitHubRateLimit {
                    retry_after: "60".to_owned(),
                    rate_limit_reset: None
                }
            );
        }

        let mut repeated = HeaderMap::new();
        repeated.append(header::RETRY_AFTER, HeaderValue::from_static("1"));
        repeated.append(header::RETRY_AFTER, HeaderValue::from_static("2"));
        repeated.append("x-ratelimit-reset", HeaderValue::from_static("101"));
        repeated.append("x-ratelimit-reset", HeaderValue::from_static("102"));
        assert_eq!(
            rate_limit_fields(
                &repeated,
                GitHubClock::Fixed {
                    seconds: 100,
                    nanos: 0
                }
            ),
            GitHubRateLimit {
                retry_after: "60".to_owned(),
                rate_limit_reset: None
            }
        );
    }

    #[tokio::test]
    async fn status_mapping_is_exact_and_quota_hints_apply_only_to_403_or_429() {
        let forbidden = response(
            StatusCode::FORBIDDEN,
            json!({"message": "secondary limit details must not be read"}),
        );
        let reads = Arc::clone(&forbidden.body_reads);
        assert!(matches!(
            HttpGitHubService::with_mock(MockGitHubTransport::new(vec![forbidden]))
                .get_owner("octocat")
                .await,
            Err(GitHubServiceError::RateLimited(GitHubRateLimit {
                retry_after,
                rate_limit_reset: None
            })) if retry_after == "60"
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 0);

        for status in [StatusCode::FORBIDDEN, StatusCode::TOO_MANY_REQUESTS] {
            let mut quota = response(status, json!({"provider_secret": true}));
            let reads = Arc::clone(&quota.body_reads);
            quota
                .headers
                .insert(header::RETRY_AFTER, HeaderValue::from_static("7"));
            quota
                .headers
                .insert("x-ratelimit-reset", HeaderValue::from_static("102"));
            assert!(matches!(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![quota]))
                    .get_owner("octocat")
                    .await,
                Err(GitHubServiceError::RateLimited(GitHubRateLimit {
                    retry_after,
                    rate_limit_reset: Some(reset)
                })) if retry_after == "7" && reset == "102"
            ));
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }

        for status in [
            StatusCode::CREATED,
            StatusCode::NO_CONTENT,
            StatusCode::UNAUTHORIZED,
            StatusCode::GONE,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
        ] {
            let mut unexpected = response(status, json!({"provider_secret": true}));
            let reads = Arc::clone(&unexpected.body_reads);
            unexpected
                .headers
                .insert(header::RETRY_AFTER, HeaderValue::from_static("7"));
            unexpected
                .headers
                .insert("x-ratelimit-reset", HeaderValue::from_static("102"));
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![unexpected]))
                    .get_owner("octocat")
                    .await,
                GitHubUpstreamErrorKind::Status,
            );
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn provider_links_support_multiple_fields_quoted_commas_and_numeric_paths() {
        let mut page = response(StatusCode::OK, json!([repository_summary_json(1, "repo")]));
        page.headers.append(
            header::LINK,
            HeaderValue::from_static(
                "<https://api.github.test/user/7/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=4>; title=\"next,page\"; rel=\"next\"",
            ),
        );
        page.headers.append(
            header::LINK,
            HeaderValue::from_static(
                "<https://evil.example/repos?page=9>; anchor=\"/alternate\"; rel=\"next\", </users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=1>; rel=prev",
            ),
        );
        let transport = MockGitHubTransport::new(vec![page]);
        let result = HttpGitHubService::with_mock(transport.clone())
            .list_repositories("octocat", 20, Some(GitHubPagination::Numbered(2)))
            .await
            .expect("provider links");
        assert_eq!(result.next.as_deref(), Some("4"));
        assert_eq!(result.prev.as_deref(), Some("1"));
        assert_eq!(transport.requests.lock().expect("requests").len(), 1);
    }

    #[tokio::test]
    async fn provider_links_pin_the_terminal_numeric_resource_identity() {
        let query = "type=owner&sort=full_name&direction=asc&per_page=20";
        for (target_id, accepted) in [(1, true), (2, false)] {
            let mut page = response(StatusCode::OK, json!([repository_summary_json(1, "repo")]));
            page.headers.insert(
                header::LINK,
                HeaderValue::from_str(&format!(
                    "</user/{target_id}/repos?{query}&page=2>; rel=next"
                ))
                .expect("link"),
            );
            let transport = MockGitHubTransport::new(vec![
                redirect_response(StatusCode::FOUND, Some(&format!("/user/1/repos?{query}"))),
                page,
            ]);
            let result = HttpGitHubService::with_mock(transport.clone())
                .list_repositories("octocat", 20, None)
                .await;
            if accepted {
                assert_eq!(
                    result.expect("same-identity provider link").next.as_deref(),
                    Some("2")
                );
            } else {
                assert_upstream(result, GitHubUpstreamErrorKind::Pagination);
            }
            assert_eq!(transport.requests.lock().expect("requests").len(), 2);
        }

        let mut page = response(StatusCode::OK, json!([tag_json("v1")]));
        page.headers.insert(
            header::LINK,
            HeaderValue::from_static("</repositories/2/tags?per_page=20&page=2>; rel=next"),
        );
        let transport = MockGitHubTransport::new(vec![
            redirect_response(StatusCode::FOUND, Some("/repositories/1/tags?per_page=20")),
            page,
        ]);
        assert_upstream(
            HttpGitHubService::with_mock(transport.clone())
                .list_tags("octocat", "repo", 20, None)
                .await,
            GitHubUpstreamErrorKind::Pagination,
        );
        assert_eq!(transport.requests.lock().expect("requests").len(), 2);
    }

    #[tokio::test]
    async fn activity_provider_links_cover_each_direction_and_empty_page_rule() {
        let mut next = response(StatusCode::OK, json!([activity_json(1)]));
        next.headers.insert(
            header::LINK,
            HeaderValue::from_static(
                "</repos/octocat/repo/activity?direction=desc&per_page=20&after=next>; rel=next",
            ),
        );
        let page = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![next]))
            .list_activity("octocat", "repo", 20, None)
            .await
            .expect("next link");
        assert_eq!(page.next.as_deref(), Some("next"));

        for (current_value, target_value) in [("same", "same"), ("current", "older")] {
            let mut prev = response(
                StatusCode::OK,
                if current_value == "current" {
                    json!([])
                } else {
                    json!([activity_json(1)])
                },
            );
            prev.headers.insert(
                header::LINK,
                HeaderValue::from_str(&format!(
                    "</repos/octocat/repo/activity?direction=desc&per_page=20&before={target_value}>; rel=prev"
                ))
                .expect("link"),
            );
            let page = HttpGitHubService::with_mock(MockGitHubTransport::new(vec![prev]))
                .list_activity(
                    "octocat",
                    "repo",
                    20,
                    Some(GitHubPagination::Activity {
                        direction: CursorDirection::Next,
                        value: current_value.to_owned(),
                    }),
                )
                .await
                .expect("previous link");
            assert_eq!(page.prev.as_deref(), Some(target_value));
        }

        let mut empty_next = response(StatusCode::OK, json!([]));
        empty_next.headers.insert(
            header::LINK,
            HeaderValue::from_static(
                "</repos/octocat/repo/activity?direction=desc&per_page=20&after=next>; rel=next",
            ),
        );
        assert_upstream(
            HttpGitHubService::with_mock(MockGitHubTransport::new(vec![empty_next]))
                .list_activity("octocat", "repo", 20, None)
                .await,
            GitHubUpstreamErrorKind::Pagination,
        );
    }

    #[test]
    fn link_field_splitting_rejects_independent_quote_and_angle_failures() {
        assert_upstream(
            split_link_values("</resource>; title=\"unterminated"),
            GitHubUpstreamErrorKind::Pagination,
        );
        assert_upstream(
            split_link_values("<https://api.github.test/resource"),
            GitHubUpstreamErrorKind::Pagination,
        );
        assert_eq!(
            split_link_values("</one>; title=\"one,two\", </two>; rel=next").expect("valid field"),
            ["</one>; title=\"one,two\"", "</two>; rel=next"]
        );
    }

    #[tokio::test]
    async fn malformed_or_nonprogressing_provider_links_fail_closed() {
        let invalid_links = [
            "<https://other.example/users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=3>; rel=next",
            "</users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=3>; rel=\"next next\"",
            "</users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=3>; rel=next, </user/1/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=4>; rel=next",
            "</users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=2>; rel=next",
            "</users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=0>; rel=prev",
            "</users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=19&page=3>; rel=next",
            "</repositories/1/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=3>; rel=next",
            "</users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20>; rel=next",
            "<>; rel=next",
        ];
        for link in invalid_links {
            let mut page = response(StatusCode::OK, json!([repository_summary_json(1, "repo")]));
            page.headers.insert(
                header::LINK,
                HeaderValue::from_str(link).expect("link field"),
            );
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![page]))
                    .list_repositories("octocat", 20, Some(GitHubPagination::Numbered(2)))
                    .await,
                GitHubUpstreamErrorKind::Pagination,
            );
        }

        let mut empty = response(StatusCode::OK, json!([]));
        empty.headers.insert(
            header::LINK,
            HeaderValue::from_static(
                "</users/octocat/repos?type=owner&sort=full_name&direction=asc&per_page=20&page=3>; rel=next",
            ),
        );
        assert_upstream(
            HttpGitHubService::with_mock(MockGitHubTransport::new(vec![empty]))
                .list_repositories("octocat", 20, Some(GitHubPagination::Numbered(2)))
                .await,
            GitHubUpstreamErrorKind::Pagination,
        );

        for link in [
            "</repos/octocat/repo/activity?direction=desc&per_page=20&before=older>; rel=prev",
            "</repos/octocat/repo/activity?direction=desc&per_page=20&after=current>; rel=next",
            "</repos/octocat/repo/activity?direction=desc&per_page=20&before=new>; rel=next",
        ] {
            let mut page = response(StatusCode::OK, json!([activity_json(1)]));
            page.headers.insert(
                header::LINK,
                HeaderValue::from_str(link).expect("link field"),
            );
            assert_upstream(
                HttpGitHubService::with_mock(MockGitHubTransport::new(vec![page]))
                    .list_activity(
                        "octocat",
                        "repo",
                        20,
                        if link.contains("current") {
                            Some(GitHubPagination::Activity {
                                direction: CursorDirection::Next,
                                value: "current".to_owned(),
                            })
                        } else {
                            None
                        },
                    )
                    .await,
                GitHubUpstreamErrorKind::Pagination,
            );
        }
    }

    #[test]
    fn projection_rejects_private_repositories_duplicate_topics_and_submillisecond_time() {
        let base = json!({
            "id": 1,
            "name": "repo",
            "full_name": "owner/repo",
            "description": null,
            "html_url": "https://github.example.test/owner/repo",
            "fork": false,
            "private": false,
            "visibility": "public",
            "language": null,
            "stargazers_count": 1,
            "forks_count": 2,
            "open_issues_count": 3,
            "archived": false,
            "created_at": "2020-01-01T00:00:00.000Z",
            "updated_at": "2020-01-02T00:00:00.000Z",
            "pushed_at": null,
            "default_branch": "main",
            "license": null,
            "topics": ["one", "one"],
            "disabled": false
        });
        let raw: RawRepository = serde_json::from_value(base.clone()).expect("raw");
        assert!(project_repository(raw).is_err());
        let mut private = base.clone();
        private["topics"] = json!([]);
        private["private"] = json!(true);
        assert!(project_repository(serde_json::from_value(private).unwrap()).is_err());
        let mut timestamp = base;
        timestamp["topics"] = json!([]);
        timestamp["created_at"] = json!("2020-01-01T00:00:00.0001Z");
        assert!(project_repository(serde_json::from_value(timestamp).unwrap()).is_err());
    }

    #[tokio::test]
    async fn overall_deadline_maps_to_timeout() {
        let mut slow = response(StatusCode::OK, json!({}));
        slow.delay = Duration::from_secs(11);
        let transport = MockGitHubTransport::new(vec![slow]);
        let service = HttpGitHubService::with_mock(transport.clone());
        tokio::time::pause();
        let task = tokio::spawn(async move { service.get_owner("octocat").await });
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(matches!(
            task.await.expect("task"),
            Err(GitHubServiceError::Timeout)
        ));
        assert_eq!(transport.requests.lock().expect("requests").len(), 1);
    }

    #[tokio::test]
    async fn redirects_share_one_deadline_and_caller_cancellation_stops_work() {
        tokio::time::pause();
        let mut first = redirect_response(StatusCode::FOUND, Some("/user/1"));
        first.delay = Duration::from_secs(6);
        let mut second = response(StatusCode::OK, owner_json());
        second.delay = Duration::from_secs(6);
        let transport = MockGitHubTransport::new(vec![first, second]);
        let service = HttpGitHubService::with_mock(transport.clone());
        let task = tokio::spawn(async move { service.get_owner("octocat").await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(matches!(
            task.await.expect("task"),
            Err(GitHubServiceError::Timeout)
        ));
        assert_eq!(transport.requests.lock().expect("requests").len(), 2);

        let mut canceled_response = response(StatusCode::OK, owner_json());
        let reads = Arc::clone(&canceled_response.body_reads);
        canceled_response.delay = Duration::from_secs(30);
        let canceled_transport = MockGitHubTransport::new(vec![canceled_response]);
        let canceled_service = HttpGitHubService::with_mock(canceled_transport.clone());
        let canceled = tokio::spawn(async move { canceled_service.get_owner("octocat").await });
        tokio::task::yield_now().await;
        assert_eq!(
            canceled_transport.requests.lock().expect("requests").len(),
            1
        );
        canceled.abort();
        assert!(canceled.await.expect_err("canceled task").is_cancelled());
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(
            canceled_transport.requests.lock().expect("requests").len(),
            1
        );
    }
}
