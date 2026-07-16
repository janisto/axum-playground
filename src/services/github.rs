use std::{collections::BTreeMap, sync::Arc};

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use utoipa::ToSchema;

const DEFAULT_BASE_URL: &str = "https://api.github.com";
const DEFAULT_USER_AGENT: &str = "axum-playground/0.1.0";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";
const GITHUB_API_VERSION: &str = "2022-11-28";
const LIST_LIMIT: usize = 30;

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
    client: Client,
    base_url: String,
    token: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MockGitHubService {
    owner: Option<Owner>,
    repos: Vec<RepoSummary>,
    repo: Option<Repo>,
    activity_page: ActivityPage,
    languages: Vec<Language>,
    tags: Vec<Tag>,
    error: Option<GitHubServiceError>,
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
    pub actor: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub timestamp: String,
    #[serde(rename = "activityType")]
    pub activity_type: String,
    #[serde(rename = "actorAvatarUrl")]
    pub actor_avatar_url: String,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubServiceError {
    NotFound,
    Forbidden,
    RateLimited,
    Upstream(GitHubUpstreamError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubUpstreamError {
    pub kind: GitHubUpstreamErrorKind,
    pub status: u16,
    pub retry_after: Option<String>,
    pub rate_limit_reset: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubUpstreamErrorKind {
    NotFound,
    Forbidden,
    RateLimited,
    Upstream,
}

impl GitHubService {
    pub fn http(token: Option<String>) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Http(HttpGitHubService {
                client: Client::new(),
                base_url: DEFAULT_BASE_URL.to_string(),
                token,
            })),
        }
    }

    pub fn mock(mock: MockGitHubService) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Mock(Box::new(mock))),
        }
    }

    #[cfg(test)]
    pub(crate) fn http_with_base_url(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            inner: Arc::new(GitHubServiceInner::Http(HttpGitHubService {
                client: Client::new(),
                base_url: base_url.into(),
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

    pub async fn list_repos(&self, owner: &str) -> Result<Vec<RepoSummary>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.list_repos(owner).await,
            GitHubServiceInner::Mock(service) => service.list_repos(owner).await,
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

    pub async fn list_tags(&self, owner: &str, repo: &str) -> Result<Vec<Tag>, GitHubServiceError> {
        match self.inner.as_ref() {
            GitHubServiceInner::Http(service) => service.list_tags(owner, repo).await,
            GitHubServiceInner::Mock(service) => service.list_tags(owner, repo).await,
        }
    }
}

impl MockGitHubService {
    pub fn demo() -> Self {
        Self {
            owner: Some(Owner {
                login: "octocat".to_string(),
                name: "The Octocat".to_string(),
                avatar_url: "https://avatars.githubusercontent.com/u/583231".to_string(),
                html_url: "https://github.com/octocat".to_string(),
                bio: String::new(),
                location: "San Francisco".to_string(),
                blog: "https://github.blog".to_string(),
                company: "@github".to_string(),
                created_at: "2011-01-25T18:44:36Z".to_string(),
                updated_at: "2024-06-01T00:00:00Z".to_string(),
            }),
            repos: vec![RepoSummary {
                name: "git-consortium".to_string(),
                full_name: "octocat/git-consortium".to_string(),
                description: "This repo is for demonstration purposes.".to_string(),
                html_url: "https://github.com/octocat/git-consortium".to_string(),
                language: "Ruby".to_string(),
                stars: 16,
                forks: 10,
                open_issues: 0,
                created_at: "2011-01-25T18:44:36Z".to_string(),
                updated_at: "2024-06-01T00:00:00Z".to_string(),
            }],
            repo: Some(Repo {
                repo_summary: RepoSummary {
                    name: "git-consortium".to_string(),
                    full_name: "octocat/git-consortium".to_string(),
                    description: "This repo is for demonstration purposes.".to_string(),
                    html_url: "https://github.com/octocat/git-consortium".to_string(),
                    language: "Ruby".to_string(),
                    stars: 16,
                    forks: 10,
                    open_issues: 0,
                    created_at: "2011-01-25T18:44:36Z".to_string(),
                    updated_at: "2024-06-01T00:00:00Z".to_string(),
                },
                default_branch: "master".to_string(),
                license: "MIT License".to_string(),
                topics: Vec::new(),
                archived: false,
                disabled: false,
            }),
            activity_page: ActivityPage {
                activities: vec![Activity {
                    id: 1,
                    actor: "octocat".to_string(),
                    git_ref: "refs/heads/master".to_string(),
                    timestamp: "2024-01-15T10:30:00Z".to_string(),
                    activity_type: "push".to_string(),
                    actor_avatar_url: "https://avatars.githubusercontent.com/u/583231".to_string(),
                }],
                next_cursor: String::new(),
            },
            languages: vec![Language {
                name: "Ruby".to_string(),
                bytes: 6789,
            }],
            tags: vec![Tag {
                name: "v1.0".to_string(),
                commit: TagCommit {
                    sha: "abc123".to_string(),
                },
            }],
            error: None,
        }
    }

    pub fn with_error(mut self, error: GitHubServiceError) -> Self {
        self.error = Some(error);
        self
    }

    pub fn with_activity_page(mut self, activity_page: ActivityPage) -> Self {
        self.activity_page = activity_page;
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

    async fn list_repos(&self, owner: &str) -> Result<Vec<RepoSummary>, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        if self
            .owner
            .as_ref()
            .is_some_and(|current_owner| current_owner.login == owner)
        {
            Ok(self.repos.clone())
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

    async fn list_tags(&self, owner: &str, repo: &str) -> Result<Vec<Tag>, GitHubServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        if self.repo.as_ref().is_some_and(|current_repo| {
            current_repo.repo_summary.full_name == format!("{owner}/{repo}")
        }) {
            Ok(self.tags.clone())
        } else {
            Err(GitHubServiceError::NotFound)
        }
    }
}

impl HttpGitHubService {
    async fn get_owner(&self, owner: &str) -> Result<Owner, GitHubServiceError> {
        let payload: GitHubOwnerPayload = self
            .send_json(self.client.get(self.url(&["users", owner])))
            .await?;
        Ok(payload.into_owner())
    }

    async fn list_repos(&self, owner: &str) -> Result<Vec<RepoSummary>, GitHubServiceError> {
        let payload: Vec<GitHubRepoSummaryPayload> = self
            .send_json(
                self.client
                    .get(self.url(&["users", owner, "repos"]))
                    .query(&[("per_page", LIST_LIMIT)]),
            )
            .await?;
        Ok(payload
            .into_iter()
            .map(GitHubRepoSummaryPayload::into_repo_summary)
            .collect())
    }

    async fn get_repo(&self, owner: &str, repo: &str) -> Result<Repo, GitHubServiceError> {
        let payload: GitHubRepoPayload = self
            .send_json(self.client.get(self.url(&["repos", owner, repo])))
            .await?;
        Ok(payload.into_repo())
    }

    async fn list_activity(
        &self,
        owner: &str,
        repo: &str,
        limit: usize,
        after_cursor: &str,
    ) -> Result<ActivityPage, GitHubServiceError> {
        let mut request = self
            .client
            .get(self.url(&["repos", owner, repo, "activity"]))
            .query(&[("per_page", limit)]);
        if !after_cursor.is_empty() {
            request = request.query(&[("after", after_cursor)]);
        }

        let response = self.send(request).await?;
        let next_cursor = response
            .headers()
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(extract_next_cursor);
        let payload = response
            .json::<Vec<GitHubActivityPayload>>()
            .await
            .map_err(|_| GitHubServiceError::Upstream(GitHubUpstreamError::upstream(0)))?;

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
            .send_json(
                self.client
                    .get(self.url(&["repos", owner, repo, "languages"])),
            )
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

    async fn list_tags(&self, owner: &str, repo: &str) -> Result<Vec<Tag>, GitHubServiceError> {
        let payload: Vec<GitHubTagPayload> = self
            .send_json(
                self.client
                    .get(self.url(&["repos", owner, repo, "tags"]))
                    .query(&[("per_page", LIST_LIMIT)]),
            )
            .await?;

        Ok(payload
            .into_iter()
            .map(GitHubTagPayload::into_tag)
            .collect())
    }

    async fn send_json<T>(&self, request: reqwest::RequestBuilder) -> Result<T, GitHubServiceError>
    where
        T: DeserializeOwned,
    {
        let response = self.send(request).await?;
        response
            .json::<T>()
            .await
            .map_err(|_| GitHubServiceError::Upstream(GitHubUpstreamError::upstream(0)))
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, GitHubServiceError> {
        let mut request = request
            .header(reqwest::header::ACCEPT, GITHUB_ACCEPT)
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .timeout(std::time::Duration::from_secs(10));

        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .await
            .map_err(|_| GitHubServiceError::Upstream(GitHubUpstreamError::upstream(0)))?;

        if response.status().is_success() {
            Ok(response)
        } else {
            Err(map_http_error(response.status(), response.headers()))
        }
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

impl GitHubUpstreamError {
    fn upstream(status: u16) -> Self {
        Self {
            kind: GitHubUpstreamErrorKind::Upstream,
            status,
            retry_after: None,
            rate_limit_reset: None,
        }
    }
}

fn map_http_error(status: StatusCode, headers: &reqwest::header::HeaderMap) -> GitHubServiceError {
    let retry_after = header_value(headers, reqwest::header::RETRY_AFTER.as_str());
    let rate_limit_reset = header_value(headers, "x-ratelimit-reset");
    let rate_limit_remaining = header_value(headers, "x-ratelimit-remaining");

    if status == StatusCode::NOT_FOUND {
        return GitHubServiceError::Upstream(GitHubUpstreamError {
            kind: GitHubUpstreamErrorKind::NotFound,
            status: status.as_u16(),
            retry_after,
            rate_limit_reset,
        });
    }

    if status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (retry_after.is_some() || rate_limit_remaining.as_deref() == Some("0")))
    {
        return GitHubServiceError::Upstream(GitHubUpstreamError {
            kind: GitHubUpstreamErrorKind::RateLimited,
            status: status.as_u16(),
            retry_after,
            rate_limit_reset,
        });
    }

    if status == StatusCode::FORBIDDEN {
        return GitHubServiceError::Upstream(GitHubUpstreamError {
            kind: GitHubUpstreamErrorKind::Forbidden,
            status: status.as_u16(),
            retry_after,
            rate_limit_reset,
        });
    }

    GitHubServiceError::Upstream(GitHubUpstreamError {
        kind: GitHubUpstreamErrorKind::Upstream,
        status: status.as_u16(),
        retry_after,
        rate_limit_reset,
    })
}

fn header_value(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn extract_next_cursor(link_header: &str) -> Option<String> {
    link_header.split(',').find_map(|part| {
        let part = part.trim();
        if !part.contains("rel=\"next\"") {
            return None;
        }

        let start = part.find('<')? + 1;
        let end = part[start..].find('>')? + start;
        let url = reqwest::Url::parse(&part[start..end]).ok()?;
        url.query_pairs()
            .find(|(key, _)| key == "after")
            .map(|(_, value)| value.into_owned())
    })
}

#[derive(Debug, Deserialize)]
struct GitHubOwnerPayload {
    login: String,
    name: Option<String>,
    avatar_url: String,
    html_url: String,
    bio: Option<String>,
    location: Option<String>,
    blog: Option<String>,
    company: Option<String>,
    created_at: String,
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
    html_url: String,
    language: Option<String>,
    stargazers_count: i32,
    forks_count: i32,
    open_issues_count: i32,
    created_at: String,
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
    actor: String,
    #[serde(rename = "ref")]
    git_ref: String,
    timestamp: String,
    activity_type: String,
    actor_avatar_url: String,
}

impl GitHubActivityPayload {
    fn into_activity(self) -> Activity {
        Activity {
            id: self.id,
            actor: self.actor,
            git_ref: self.git_ref,
            timestamp: self.timestamp,
            activity_type: self.activity_type,
            actor_avatar_url: self.actor_avatar_url,
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
    use axum::{
        Json, Router,
        extract::Query,
        http::{HeaderMap, StatusCode, header},
        routing::get,
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use super::{
        GITHUB_ACCEPT, GitHubService, GitHubServiceError, GitHubServiceInner,
        GitHubUpstreamErrorKind,
    };

    async fn spawn_test_server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });

        (format!("http://{address}"), handle)
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

    #[tokio::test]
    async fn http_service_parses_owner_and_sets_required_headers() {
        let app = Router::new().route(
            "/users/{owner}",
            get(|headers: HeaderMap| async move {
                assert_eq!(
                    headers
                        .get(header::ACCEPT)
                        .and_then(|value| value.to_str().ok()),
                    Some(GITHUB_ACCEPT)
                );
                assert!(headers.contains_key(header::USER_AGENT));
                assert_eq!(
                    headers
                        .get("x-github-api-version")
                        .and_then(|value| value.to_str().ok()),
                    Some("2022-11-28")
                );

                Json(json!({
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
                }))
            }),
        );

        let (base_url, handle) = spawn_test_server(app).await;
        let service = GitHubService::http_with_base_url(base_url, Some("token".to_string()));
        let owner = service
            .get_owner("octocat")
            .await
            .expect("owner should load");
        handle.abort();

        assert_eq!(owner.company, "@github");
        assert_eq!(owner.location, "San Francisco");
    }

    #[tokio::test]
    async fn http_service_maps_rate_limits_and_extracts_headers() {
        let app = Router::new().route(
            "/users/{owner}",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    [
                        (header::RETRY_AFTER, "60"),
                        (
                            header::HeaderName::from_static("x-ratelimit-reset"),
                            "1700000000",
                        ),
                        (
                            header::HeaderName::from_static("x-ratelimit-remaining"),
                            "0",
                        ),
                    ],
                    Json(json!({"message": "rate limited"})),
                )
            }),
        );

        let (base_url, handle) = spawn_test_server(app).await;
        let service = GitHubService::http_with_base_url(base_url, None);
        let error = service
            .get_owner("octocat")
            .await
            .expect_err("request should fail");
        handle.abort();

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
    async fn http_service_extracts_next_activity_cursor() {
        let app = Router::new().route(
            "/repos/{owner}/{repo}/activity",
            get(|Query(query): Query<std::collections::HashMap<String, String>>| async move {
                assert_eq!(query.get("per_page").map(String::as_str), Some("10"));

                (
                    [(
                        header::LINK,
                        "<https://api.github.com/repos/octocat/git-consortium/activity?after=abc123>; rel=\"next\"",
                    )],
                    Json(json!([
                        {
                            "id": 1,
                            "actor": "octocat",
                            "ref": "refs/heads/master",
                            "timestamp": "2024-01-15T10:30:00Z",
                            "activity_type": "push",
                            "actor_avatar_url": "https://avatars.githubusercontent.com/u/583231"
                        }
                    ])),
                )
            }),
        );

        let (base_url, handle) = spawn_test_server(app).await;
        let service = GitHubService::http_with_base_url(base_url, None);
        let page = service
            .list_activity("octocat", "git-consortium", 10, "")
            .await
            .expect("activity should load");
        handle.abort();

        assert_eq!(page.next_cursor, "abc123");
        assert_eq!(page.activities.len(), 1);
    }

    #[tokio::test]
    async fn http_service_sorts_languages_by_byte_count() {
        let app = Router::new().route(
            "/repos/{owner}/{repo}/languages",
            get(|| async {
                Json(Value::Object(
                    json!({"Ruby": 6789, "Go": 12345})
                        .as_object()
                        .expect("object")
                        .clone(),
                ))
            }),
        );

        let (base_url, handle) = spawn_test_server(app).await;
        let service = GitHubService::http_with_base_url(base_url, None);
        let languages = service
            .list_languages("octocat", "git-consortium")
            .await
            .expect("languages should load");
        handle.abort();

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
