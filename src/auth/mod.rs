use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::Response,
};
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder as CredentialsBuilder};
use jsonwebtoken::{
    Algorithm, DecodingKey, Header, Validation,
    dangerous::insecure_decode,
    decode, decode_header,
    errors::{Error as JwtError, ErrorKind as JwtErrorKind},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, warn};

use crate::{config::AppConfig, error::StartupError, problem::problem_response, state::AppState};

const CERT_RETRY_AFTER_SECS: &str = "30";
const GOOGLE_JWKS_URL: &str =
    "https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com";
const IDENTITY_LOOKUP_URL: &str = "https://identitytoolkit.googleapis.com/v1/accounts:lookup";
const IDENTITY_TOOLKIT_SCOPE: &str = "https://www.googleapis.com/auth/identitytoolkit";
const DEFAULT_USER_AGENT: &str = "axum-playground/0.1.0";
const DEFAULT_JWKS_TTL: Duration = Duration::from_secs(3600);
const JWKS_RETRY_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_UNKNOWN_KEY_RETRIES: usize = 256;
const JWT_LEEWAY_SECS: u64 = 60;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FirebaseUser {
    pub uid: String,
    pub email: String,
    pub email_verified: bool,
}

impl FirebaseUser {
    pub fn new(uid: impl Into<String>, email: impl Into<String>, email_verified: bool) -> Self {
        Self {
            uid: uid.into(),
            email: email.into(),
            email_verified,
        }
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum AuthError {
    #[error("missing authorization header")]
    MissingAuthorization,
    #[error("invalid authorization header")]
    InvalidAuthorization,
    #[error("invalid token")]
    InvalidToken,
    #[error("token expired")]
    TokenExpired,
    #[error("token revoked")]
    TokenRevoked,
    #[error("user disabled")]
    UserDisabled,
    #[error("failed to fetch certificates")]
    CertificateFetch,
    #[error("authentication service unavailable")]
    ServiceUnavailable,
}

#[derive(Clone, Debug)]
pub struct AuthVerifier {
    inner: Arc<AuthVerifierInner>,
}

#[derive(Clone, Debug)]
enum AuthVerifierInner {
    Production(Box<ProductionAuthVerifier>),
    Emulator(Box<EmulatorAuthVerifier>),
    Mock(Box<MockAuthVerifier>),
}

#[derive(Clone, Debug)]
struct ProductionAuthVerifier {
    project_id: String,
    jwks_client: GoogleJwksClient,
    lookup_client: IdentityPlatformLookupClient,
}

#[derive(Clone, Debug)]
struct GoogleJwksClient {
    transport: GoogleJwksTransport,
    url: String,
    state: Arc<RwLock<JwksState>>,
    refresh: Arc<Mutex<()>>,
}

#[derive(Clone, Debug)]
enum GoogleJwksTransport {
    Http(Client),
    #[cfg(test)]
    Mock(MockGoogleJwksTransport),
}

#[derive(Clone, Debug)]
struct GoogleJwksResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct MockGoogleJwksTransport {
    responses: Arc<Mutex<std::collections::VecDeque<Result<GoogleJwksResponse, AuthError>>>>,
    fetches: Arc<std::sync::atomic::AtomicUsize>,
}

#[derive(Clone, Debug)]
struct EmulatorAuthVerifier {
    project_id: String,
}

#[derive(Clone, Debug)]
pub struct MockAuthVerifier {
    user: FirebaseUser,
    error: Option<AuthError>,
}

#[derive(Clone, Debug)]
struct CachedJwks {
    keys: HashMap<String, GoogleJwk>,
    expires_at: Instant,
}

#[derive(Clone, Debug, Default)]
struct JwksState {
    cached: Option<CachedJwks>,
    fetch_retry: Option<(Instant, AuthError)>,
    unknown_key_retries: HashMap<String, Instant>,
}

#[derive(Clone, Debug)]
struct IdentityPlatformLookupClient {
    client: Client,
    credentials: AccessTokenCredentials,
    project_id: String,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedUser(pub FirebaseUser);

#[derive(Clone, Debug, Deserialize)]
struct FirebaseClaims {
    sub: String,
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
    exp: Option<u64>,
    iat: Option<u64>,
    auth_time: Option<u64>,
    iss: Option<String>,
    aud: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct GoogleJwkSet {
    keys: Vec<GoogleJwk>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct GoogleJwk {
    kid: String,
    n: String,
    e: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentityLookupRequest {
    local_id: Vec<String>,
    target_project_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct IdentityLookupResponse {
    #[serde(default)]
    users: Vec<IdentityLookupUser>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityLookupUser {
    local_id: String,
    #[serde(default)]
    disabled: bool,
    valid_since: Option<String>,
}

impl AuthVerifier {
    pub fn from_config(config: &AppConfig) -> Result<Self, StartupError> {
        let project_id = config.firebase_project_id.clone();

        if let Some(host) = config.firebase_auth_emulator_host.as_deref() {
            if !config.emulator_host_is_loopback(host) {
                return Err(StartupError::UnsafeEmulatorHost {
                    variable: "FIREBASE_AUTH_EMULATOR_HOST",
                    host: host.to_owned(),
                });
            }
            return Ok(Self {
                inner: Arc::new(AuthVerifierInner::Emulator(Box::new(
                    EmulatorAuthVerifier { project_id },
                ))),
            });
        }

        let credentials = CredentialsBuilder::default()
            .with_scopes([IDENTITY_TOOLKIT_SCOPE])
            .build_access_token_credentials()
            .map_err(|error| StartupError::AuthInitialization(error.to_string()))?;

        let client = Client::builder()
            .user_agent(DEFAULT_USER_AGENT)
            .build()
            .map_err(|error| StartupError::AuthInitialization(error.to_string()))?;

        Ok(Self {
            inner: Arc::new(AuthVerifierInner::Production(Box::new(
                ProductionAuthVerifier {
                    project_id: project_id.clone(),
                    jwks_client: GoogleJwksClient::new(client.clone(), GOOGLE_JWKS_URL),
                    lookup_client: IdentityPlatformLookupClient {
                        client,
                        credentials,
                        project_id,
                    },
                },
            ))),
        })
    }

    #[must_use]
    pub fn mock(mock: MockAuthVerifier) -> Self {
        Self {
            inner: Arc::new(AuthVerifierInner::Mock(Box::new(mock))),
        }
    }

    pub async fn verify(&self, token: &str) -> Result<FirebaseUser, AuthError> {
        match self.inner.as_ref() {
            AuthVerifierInner::Production(verifier) => verifier.verify(token).await,
            AuthVerifierInner::Emulator(verifier) => verifier.verify(token).await,
            AuthVerifierInner::Mock(verifier) => verifier.verify(token).await,
        }
    }
}

impl MockAuthVerifier {
    #[must_use]
    pub fn allow(user: FirebaseUser) -> Self {
        Self { user, error: None }
    }

    #[must_use]
    pub fn test_user() -> Self {
        Self::allow(FirebaseUser::new("user-123", "test@example.com", true))
    }

    #[must_use]
    pub fn with_error(mut self, error: AuthError) -> Self {
        self.error = Some(error);
        self
    }

    async fn verify(&self, _token: &str) -> Result<FirebaseUser, AuthError> {
        self.error
            .clone()
            .map_or_else(|| Ok(self.user.clone()), Err)
    }
}

impl ProductionAuthVerifier {
    async fn verify(&self, token: &str) -> Result<FirebaseUser, AuthError> {
        let header = decode_header(token).map_err(map_jwt_error)?;
        let kid = rsa_key_id(&header)?;

        let jwk = self.jwks_client.key_for(kid).await?;
        let key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
            .map_err(|_| AuthError::InvalidToken)?;

        let claims = decode::<FirebaseClaims>(token, &key, &firebase_validation(&self.project_id))
            .map(|data| data.claims)
            .map_err(map_jwt_error)?;

        validate_common_claims(&claims, &self.project_id)?;

        let lookup = self.lookup_client.lookup_user(&claims.sub).await?;
        if lookup.disabled {
            return Err(AuthError::UserDisabled);
        }

        let valid_since = lookup.valid_since_epoch()?;
        let auth_time = claims.auth_time.ok_or(AuthError::InvalidToken)?;
        if token_is_revoked(valid_since, auth_time) {
            return Err(AuthError::TokenRevoked);
        }

        Ok(claims.into_user())
    }
}

impl GoogleJwksClient {
    fn new(client: Client, url: impl Into<String>) -> Self {
        Self {
            transport: GoogleJwksTransport::Http(client),
            url: url.into(),
            state: Arc::new(RwLock::new(JwksState::default())),
            refresh: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    fn with_mock_transport(transport: MockGoogleJwksTransport) -> Self {
        Self {
            transport: GoogleJwksTransport::Mock(transport),
            url: GOOGLE_JWKS_URL.to_owned(),
            state: Arc::new(RwLock::new(JwksState::default())),
            refresh: Arc::new(Mutex::new(())),
        }
    }

    async fn key_for(&self, kid: &str) -> Result<GoogleJwk, AuthError> {
        let now = Instant::now();
        if let Some(result) = self.state.read().await.key_or_retry(kid, now) {
            return result;
        }

        let _refresh_guard = self.refresh.lock().await;
        let now = Instant::now();
        if let Some(result) = self.state.read().await.key_or_retry(kid, now) {
            return result;
        }

        let refreshed = self.fetch().await;
        let now = Instant::now();
        let (keys, ttl) = match refreshed {
            Ok(refreshed) => refreshed,
            Err(error) => {
                let mut state = self.state.write().await;
                state.fetch_retry = Some((now + JWKS_RETRY_COOLDOWN, error.clone()));
                return Err(error);
            }
        };
        let key = keys.get(kid).cloned();
        let mut state = self.state.write().await;
        state.cached = Some(CachedJwks::new(keys, now, ttl));
        state.fetch_retry = None;
        if key.is_some() {
            state.unknown_key_retries.remove(kid);
        } else {
            state.remember_unknown_key(kid, now);
        }

        key.ok_or(AuthError::InvalidToken)
    }

    async fn fetch(&self) -> Result<(HashMap<String, GoogleJwk>, Duration), AuthError> {
        let response = self.transport.fetch(&self.url).await?;
        require_successful_response(response.status, AuthError::CertificateFetch)?;

        let ttl = cache_ttl(&response.headers).unwrap_or(DEFAULT_JWKS_TTL);
        let payload: GoogleJwkSet =
            serde_json::from_slice(&response.body).map_err(|_| AuthError::CertificateFetch)?;
        let keys = payload
            .keys
            .into_iter()
            .map(|key| (key.kid.clone(), key))
            .collect();

        Ok((keys, ttl))
    }
}

impl GoogleJwksTransport {
    async fn fetch(&self, url: &str) -> Result<GoogleJwksResponse, AuthError> {
        match self {
            Self::Http(client) => {
                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|_| AuthError::CertificateFetch)?;
                let status = response.status();
                let headers = response.headers().clone();
                let body = if status.is_success() {
                    response
                        .bytes()
                        .await
                        .map_err(|_| AuthError::CertificateFetch)?
                        .to_vec()
                } else {
                    Vec::new()
                };
                Ok(GoogleJwksResponse {
                    status,
                    headers,
                    body,
                })
            }
            #[cfg(test)]
            Self::Mock(transport) => transport.fetch().await,
        }
    }
}

#[cfg(test)]
impl MockGoogleJwksTransport {
    fn new(responses: Vec<Result<GoogleJwksResponse, AuthError>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            fetches: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    async fn fetch(&self) -> Result<GoogleJwksResponse, AuthError> {
        self.fetches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.responses
            .lock()
            .await
            .pop_front()
            .expect("mock JWKS response should be configured")
    }

    fn fetch_count(&self) -> usize {
        self.fetches.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl JwksState {
    fn key_or_retry(&self, kid: &str, now: Instant) -> Option<Result<GoogleJwk, AuthError>> {
        if let Some(key) = self
            .cached
            .as_ref()
            .and_then(|cache| cache.key_at(kid, now))
        {
            return Some(Ok(key));
        }
        if let Some(result) = self
            .fetch_retry
            .as_ref()
            .filter(|(retry_after, _)| now < *retry_after)
            .map(|(_, error)| Err(error.clone()))
        {
            return Some(result);
        }
        self.unknown_key_retries
            .get(kid)
            .filter(|retry_after| now < **retry_after)
            .map(|_| Err(AuthError::InvalidToken))
    }

    fn remember_unknown_key(&mut self, kid: &str, now: Instant) {
        self.unknown_key_retries
            .retain(|_, retry_after| now < *retry_after);
        if self.unknown_key_retries.len() >= MAX_UNKNOWN_KEY_RETRIES
            && !self.unknown_key_retries.contains_key(kid)
            && let Some(oldest_kid) = self
                .unknown_key_retries
                .iter()
                .min_by_key(|(_, retry_after)| **retry_after)
                .map(|(kid, _)| kid.clone())
        {
            self.unknown_key_retries.remove(&oldest_kid);
        }
        self.unknown_key_retries
            .insert(kid.to_owned(), now + JWKS_RETRY_COOLDOWN);
    }
}

impl CachedJwks {
    fn new(keys: HashMap<String, GoogleJwk>, now: Instant, ttl: Duration) -> Self {
        Self {
            keys,
            expires_at: now + ttl,
        }
    }

    fn key_at(&self, kid: &str, now: Instant) -> Option<GoogleJwk> {
        if !self.is_fresh(now) {
            return None;
        }

        self.keys.get(kid).cloned()
    }

    fn is_fresh(&self, now: Instant) -> bool {
        now < self.expires_at
    }
}

impl EmulatorAuthVerifier {
    async fn verify(&self, token: &str) -> Result<FirebaseUser, AuthError> {
        let claims = insecure_decode::<FirebaseClaims>(token)
            .map(|data| data.claims)
            .map_err(map_jwt_error)?;

        validate_common_claims(&claims, &self.project_id)?;

        Ok(claims.into_user())
    }
}

impl IdentityPlatformLookupClient {
    async fn lookup_user(&self, uid: &str) -> Result<IdentityLookupUser, AuthError> {
        let access_token = self
            .credentials
            .access_token()
            .await
            .map_err(|_| AuthError::ServiceUnavailable)?;

        let response = self
            .client
            .post(IDENTITY_LOOKUP_URL)
            .bearer_auth(access_token.token)
            .json(&IdentityLookupRequest {
                local_id: vec![uid.to_owned()],
                target_project_id: self.project_id.clone(),
            })
            .send()
            .await
            .map_err(|_| AuthError::ServiceUnavailable)?;

        require_successful_response(response.status(), AuthError::ServiceUnavailable)?;

        let payload: IdentityLookupResponse = response
            .json()
            .await
            .map_err(|_| AuthError::ServiceUnavailable)?;

        payload.user(uid)
    }
}

impl IdentityLookupResponse {
    fn user(self, uid: &str) -> Result<IdentityLookupUser, AuthError> {
        self.users
            .into_iter()
            .find(|user| user.local_id == uid)
            .ok_or(AuthError::InvalidToken)
    }
}

impl IdentityLookupUser {
    fn valid_since_epoch(&self) -> Result<u64, AuthError> {
        self.valid_since
            .as_deref()
            .ok_or(AuthError::ServiceUnavailable)?
            .parse()
            .map_err(|_| AuthError::ServiceUnavailable)
    }
}

impl FirebaseClaims {
    fn into_user(self) -> FirebaseUser {
        FirebaseUser::new(
            self.sub,
            self.email.unwrap_or_default(),
            self.email_verified,
        )
    }
}

pub fn extract_bearer_token(header_value: &str) -> Result<String, AuthError> {
    let header_value = header_value.trim();
    if header_value.is_empty() {
        return Err(AuthError::MissingAuthorization);
    }

    let parts = header_value.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(AuthError::InvalidAuthorization);
    }

    if !parts[0].eq_ignore_ascii_case("bearer") || parts[1].is_empty() {
        return Err(AuthError::InvalidAuthorization);
    }

    Ok(parts[1].to_owned())
}

impl FromRequestParts<Arc<AppState>> for AuthenticatedUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let authorization = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                unauthorized_response(&parts.headers, "missing or invalid authorization header")
            })?;

        let token = extract_bearer_token(authorization).map_err(|error| {
            debug!(reason = %categorize_auth_error(&error), "auth failed: invalid authorization header");
            unauthorized_response(&parts.headers, "missing or invalid authorization header")
        })?;

        let user = state.auth_verifier.verify(&token).await.map_err(|error| {
            if auth_error_is_dependency_failure(&error) {
                warn!(reason = %categorize_auth_error(&error), "auth dependency failed");
            } else {
                debug!(reason = %categorize_auth_error(&error), "auth failed: token verification failed");
            }
            map_auth_error(&parts.headers, error)
        })?;

        parts.extensions.insert(user.clone());
        Ok(Self(user))
    }
}

fn validate_common_claims(claims: &FirebaseClaims, project_id: &str) -> Result<(), AuthError> {
    validate_common_claims_at(claims, project_id, unix_timestamp_now())
}

fn validate_common_claims_at(
    claims: &FirebaseClaims,
    project_id: &str,
    now: u64,
) -> Result<(), AuthError> {
    if claims.sub.trim().is_empty() {
        return Err(AuthError::InvalidToken);
    }

    let Some(issued_at) = claims.iat else {
        return Err(AuthError::InvalidToken);
    };
    let Some(auth_time) = claims.auth_time else {
        return Err(AuthError::InvalidToken);
    };
    let Some(expiration) = claims.exp else {
        return Err(AuthError::InvalidToken);
    };
    let Some(issuer) = claims.iss.as_deref() else {
        return Err(AuthError::InvalidToken);
    };
    let Some(audience) = claims.aud.as_ref() else {
        return Err(AuthError::InvalidToken);
    };

    if expiration.saturating_add(JWT_LEEWAY_SECS) < now {
        return Err(AuthError::TokenExpired);
    }
    if issued_at > now.saturating_add(JWT_LEEWAY_SECS)
        || auth_time > now.saturating_add(JWT_LEEWAY_SECS)
    {
        return Err(AuthError::InvalidToken);
    }
    if issuer != expected_issuer(project_id) {
        return Err(AuthError::InvalidToken);
    }
    if audience != project_id {
        return Err(AuthError::InvalidToken);
    }

    Ok(())
}

fn rsa_key_id(header: &Header) -> Result<&str, AuthError> {
    if header.alg != Algorithm::RS256 {
        return Err(AuthError::InvalidToken);
    }

    header.kid.as_deref().ok_or(AuthError::InvalidToken)
}

fn firebase_validation(project_id: &str) -> Validation {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = JWT_LEEWAY_SECS;
    validation.set_audience(&[project_id]);
    validation.set_issuer(&[expected_issuer(project_id)]);
    validation.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);
    validation
}

fn expected_issuer(project_id: &str) -> String {
    format!("https://securetoken.google.com/{project_id}")
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_secs()
}

fn token_is_revoked(valid_since: u64, auth_time: u64) -> bool {
    auth_time < valid_since
}

fn require_successful_response(status: StatusCode, error: AuthError) -> Result<(), AuthError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(error)
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "jsonwebtoken passes owned errors through Result::map_err"
)]
fn map_jwt_error(error: JwtError) -> AuthError {
    match error.kind() {
        JwtErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::InvalidToken,
    }
}

fn categorize_auth_error(error: &AuthError) -> &'static str {
    match error {
        AuthError::MissingAuthorization => "no_token",
        AuthError::InvalidAuthorization => "invalid_header",
        AuthError::InvalidToken => "invalid_token",
        AuthError::TokenExpired => "token_expired",
        AuthError::TokenRevoked => "token_revoked",
        AuthError::UserDisabled => "user_disabled",
        AuthError::CertificateFetch => "certificate_fetch_failed",
        AuthError::ServiceUnavailable => "service_unavailable",
    }
}

fn auth_error_is_dependency_failure(error: &AuthError) -> bool {
    matches!(
        error,
        AuthError::CertificateFetch | AuthError::ServiceUnavailable
    )
}

fn unauthorized_response(headers: &HeaderMap, detail: &str) -> Response {
    let mut response = problem_response(StatusCode::UNAUTHORIZED, detail, headers);
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn service_unavailable_response(headers: &HeaderMap, detail: &str, retry_after: bool) -> Response {
    let mut response = problem_response(StatusCode::SERVICE_UNAVAILABLE, detail, headers);
    if retry_after {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            HeaderValue::from_static(CERT_RETRY_AFTER_SECS),
        );
    }
    response
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the extractor transfers ownership of its authentication failure"
)]
fn map_auth_error(headers: &HeaderMap, error: AuthError) -> Response {
    match error {
        AuthError::CertificateFetch => service_unavailable_response(
            headers,
            "authentication service temporarily unavailable",
            true,
        ),
        AuthError::ServiceUnavailable => service_unavailable_response(
            headers,
            "authentication service temporarily unavailable",
            false,
        ),
        AuthError::MissingAuthorization | AuthError::InvalidAuthorization => {
            unauthorized_response(headers, "missing or invalid authorization header")
        }
        AuthError::InvalidToken
        | AuthError::TokenExpired
        | AuthError::TokenRevoked
        | AuthError::UserDisabled => unauthorized_response(headers, "invalid or expired token"),
    }
}

fn cache_ttl(headers: &HeaderMap) -> Option<Duration> {
    let cache_control = headers.get(header::CACHE_CONTROL)?.to_str().ok()?;

    for directive in cache_control.split(',') {
        let directive = directive.trim();
        let Some(max_age) = directive.strip_prefix("max-age=") else {
            continue;
        };
        let seconds = max_age.parse::<u64>().ok()?;
        return Some(Duration::from_secs(seconds));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        time::{Duration, Instant},
    };

    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use futures_util::future::join_all;
    use jsonwebtoken::{
        Algorithm, Header,
        errors::{Error as JwtError, ErrorKind as JwtErrorKind},
    };
    use serde_json::json;

    use super::{
        AuthError, AuthVerifier, CachedJwks, EmulatorAuthVerifier, FirebaseClaims, FirebaseUser,
        GoogleJwk, GoogleJwksClient, GoogleJwksResponse, IdentityLookupResponse,
        IdentityLookupUser, JWT_LEEWAY_SECS, JwksState, MAX_UNKNOWN_KEY_RETRIES,
        MockGoogleJwksTransport, auth_error_is_dependency_failure, cache_ttl,
        categorize_auth_error, expected_issuer, extract_bearer_token, firebase_validation,
        map_jwt_error, require_successful_response, rsa_key_id, token_is_revoked,
        validate_common_claims_at,
    };
    use crate::{config::AppConfig, error::StartupError};

    fn jwks_response(
        status: StatusCode,
        max_age: u64,
        keys: &serde_json::Value,
    ) -> GoogleJwksResponse {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_str(&format!("public, max-age={max_age}"))
                .expect("cache-control should be valid"),
        );
        GoogleJwksResponse {
            status,
            headers,
            body: serde_json::to_vec(&json!({"keys": keys}))
                .expect("JWKS payload should serialize"),
        }
    }

    #[test]
    fn extract_bearer_token_accepts_case_insensitive_scheme() {
        assert_eq!(
            extract_bearer_token("bearer token-123").expect("token should parse"),
            "token-123"
        );
        assert_eq!(
            extract_bearer_token("BEARER   token-123   ").expect("token should parse"),
            "token-123"
        );
    }

    #[test]
    fn extract_bearer_token_rejects_invalid_values() {
        assert_eq!(
            extract_bearer_token(""),
            Err(AuthError::MissingAuthorization)
        );
        assert_eq!(
            extract_bearer_token("Basic dXNlcjpwYXNz"),
            Err(AuthError::InvalidAuthorization)
        );
        assert_eq!(
            extract_bearer_token("Bearer token extra"),
            Err(AuthError::InvalidAuthorization)
        );
    }

    #[test]
    fn auth_emulator_requires_a_local_environment_and_loopback_host() {
        let mut config = test_config();
        config.firebase_auth_emulator_host = Some("emulator.example.com:9099".to_owned());
        assert!(matches!(
            AuthVerifier::from_config(&config),
            Err(StartupError::UnsafeEmulatorHost { .. })
        ));

        config.firebase_auth_emulator_host = Some("127.0.0.1:9099".to_owned());
        config.app_environment = crate::config::AppEnvironment::Production;
        assert!(matches!(
            AuthVerifier::from_config(&config),
            Err(StartupError::UnsafeEmulatorHost { .. })
        ));

        config.app_environment = crate::config::AppEnvironment::Development;
        assert!(AuthVerifier::from_config(&config).is_ok());
    }

    #[test]
    fn revocation_uses_auth_time_instead_of_token_issue_time() {
        assert!(token_is_revoked(200, 100));
        assert!(!token_is_revoked(200, 200));
    }

    #[test]
    fn revocation_metadata_fails_closed_when_missing_or_malformed() {
        let user = |valid_since: Option<&str>| IdentityLookupUser {
            local_id: "user-123".to_owned(),
            disabled: false,
            valid_since: valid_since.map(ToOwned::to_owned),
        };

        assert_eq!(user(Some("200")).valid_since_epoch(), Ok(200));
        assert_eq!(
            user(None).valid_since_epoch(),
            Err(AuthError::ServiceUnavailable)
        );
        assert_eq!(
            user(Some("not-a-timestamp")).valid_since_epoch(),
            Err(AuthError::ServiceUnavailable)
        );
    }

    #[test]
    fn production_header_policy_requires_rs256_and_a_key_id() {
        let mut valid = Header::new(Algorithm::RS256);
        valid.kid = Some("google-key".to_owned());
        assert_eq!(rsa_key_id(&valid), Ok("google-key"));

        assert_eq!(
            rsa_key_id(&Header::new(Algorithm::RS256)),
            Err(AuthError::InvalidToken)
        );

        let mut wrong_algorithm = Header::new(Algorithm::HS256);
        wrong_algorithm.kid = Some("google-key".to_owned());
        assert_eq!(rsa_key_id(&wrong_algorithm), Err(AuthError::InvalidToken));
    }

    #[test]
    fn production_validation_pins_firebase_issuer_audience_and_claims() {
        let validation = firebase_validation("demo-test-project");

        assert_eq!(validation.algorithms, [Algorithm::RS256]);
        assert_eq!(validation.leeway, JWT_LEEWAY_SECS);
        assert_eq!(
            validation
                .aud
                .as_ref()
                .and_then(|values| values.get("demo-test-project")),
            Some(&"demo-test-project".to_owned())
        );
        assert_eq!(
            validation.iss.as_ref().and_then(|values| {
                values.get("https://securetoken.google.com/demo-test-project")
            }),
            Some(&"https://securetoken.google.com/demo-test-project".to_owned())
        );
        assert_eq!(
            validation.required_spec_claims,
            ["aud", "exp", "iss", "sub"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect()
        );
    }

    #[test]
    fn common_claim_validation_enforces_exact_time_issuer_and_audience_boundaries() {
        let now = 10_000;
        let mut claims = valid_claims(now);
        claims.exp = Some(now - JWT_LEEWAY_SECS);
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Ok(())
        );

        claims.exp = Some(now - JWT_LEEWAY_SECS - 1);
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Err(AuthError::TokenExpired)
        );

        claims = valid_claims(now);
        claims.iat = Some(now + JWT_LEEWAY_SECS);
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Ok(())
        );

        claims.iat = Some(now + JWT_LEEWAY_SECS + 1);
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Err(AuthError::InvalidToken)
        );

        claims = valid_claims(now);
        claims.auth_time = Some(now + JWT_LEEWAY_SECS);
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Ok(())
        );

        claims.auth_time = Some(now + JWT_LEEWAY_SECS + 1);
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Err(AuthError::InvalidToken)
        );

        claims = valid_claims(now);
        claims.iss = Some(expected_issuer("other-project"));
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Err(AuthError::InvalidToken)
        );

        claims = valid_claims(now);
        claims.aud = Some("other-project".to_owned());
        assert_eq!(
            validate_common_claims_at(&claims, "demo-test-project", now),
            Err(AuthError::InvalidToken)
        );
    }

    #[test]
    fn cached_jwks_requires_a_matching_unexpired_key() {
        let now = Instant::now();
        let ttl = Duration::from_secs(60);
        let cache = CachedJwks::new(
            HashMap::from([(
                "google-key".to_owned(),
                GoogleJwk {
                    kid: "google-key".to_owned(),
                    n: "modulus".to_owned(),
                    e: "AQAB".to_owned(),
                },
            )]),
            now,
            ttl,
        );

        assert_eq!(cache.expires_at, now + ttl);
        assert_eq!(
            cache.key_at("google-key", now).map(|key| key.kid),
            Some("google-key".to_owned())
        );
        assert_eq!(cache.key_at("missing-key", now).map(|key| key.kid), None);
        assert!(cache.is_fresh(now));
        assert_eq!(
            cache
                .key_at("google-key", cache.expires_at)
                .map(|key| key.kid),
            None
        );
    }

    #[test]
    fn jwks_retry_cooldown_expires_at_the_exact_deadline() {
        let deadline = Instant::now();
        let state = JwksState {
            cached: None,
            fetch_retry: Some((deadline, AuthError::CertificateFetch)),
            unknown_key_retries: HashMap::from([("missing-key".to_owned(), deadline)]),
        };

        assert_eq!(
            state.key_or_retry("key", deadline - Duration::from_nanos(1)),
            Some(Err(AuthError::CertificateFetch))
        );
        assert_eq!(state.key_or_retry("key", deadline), None);

        let state = JwksState {
            cached: None,
            fetch_retry: None,
            unknown_key_retries: HashMap::from([("missing-key".to_owned(), deadline)]),
        };
        assert_eq!(
            state.key_or_retry("missing-key", deadline - Duration::from_nanos(1)),
            Some(Err(AuthError::InvalidToken))
        );
        assert_eq!(state.key_or_retry("different-key", deadline), None);
        assert_eq!(state.key_or_retry("missing-key", deadline), None);
    }

    #[test]
    fn unknown_key_retry_cache_is_bounded_and_prunes_expired_entries() {
        let now = Instant::now();
        let mut state = JwksState::default();
        for index in 0..=MAX_UNKNOWN_KEY_RETRIES {
            state.remember_unknown_key(&format!("key-{index}"), now);
        }
        assert_eq!(state.unknown_key_retries.len(), MAX_UNKNOWN_KEY_RETRIES);
        assert!(state.unknown_key_retries.contains_key("key-256"));

        state.remember_unknown_key("fresh-key", now + super::JWKS_RETRY_COOLDOWN);
        assert_eq!(state.unknown_key_retries.len(), 1);
        assert!(state.unknown_key_retries.contains_key("fresh-key"));
    }

    #[tokio::test]
    async fn jwks_unknown_keys_are_cached_and_refreshes_are_singleflight() {
        let known_keys = json!([{"kid": "known-key", "n": "modulus", "e": "AQAB"}]);
        let transport = MockGoogleJwksTransport::new(vec![
            Ok(jwks_response(StatusCode::OK, 3600, &known_keys)),
            Ok(jwks_response(StatusCode::OK, 3600, &known_keys)),
            Ok(jwks_response(StatusCode::OK, 3600, &known_keys)),
        ]);
        let client = GoogleJwksClient::with_mock_transport(transport.clone());

        assert_eq!(
            client
                .key_for("known-key")
                .await
                .expect("known key should load")
                .kid,
            "known-key"
        );
        assert_eq!(transport.fetch_count(), 1);

        assert_eq!(
            client.key_for("unknown-key").await,
            Err(AuthError::InvalidToken)
        );
        assert_eq!(
            client.key_for("unknown-key").await,
            Err(AuthError::InvalidToken)
        );
        assert_eq!(transport.fetch_count(), 2);
        assert!(client.key_for("known-key").await.is_ok());
        assert_eq!(transport.fetch_count(), 2);

        let results = join_all((0..8).map(|_| {
            let client = client.clone();
            async move { client.key_for("attacker-key").await }
        }))
        .await;
        assert!(
            results
                .iter()
                .all(|result| *result == Err(AuthError::InvalidToken))
        );
        assert_eq!(transport.fetch_count(), 3);
    }

    #[tokio::test]
    async fn unknown_key_cooldown_does_not_block_a_different_rotated_key() {
        let old_keys = json!([{"kid": "old-key", "n": "old-modulus", "e": "AQAB"}]);
        let rotated_keys = json!([
            {"kid": "old-key", "n": "old-modulus", "e": "AQAB"},
            {"kid": "new-key", "n": "new-modulus", "e": "AQAB"}
        ]);
        let transport = MockGoogleJwksTransport::new(vec![
            Ok(jwks_response(StatusCode::OK, 3600, &old_keys)),
            Ok(jwks_response(StatusCode::OK, 3600, &old_keys)),
            Ok(jwks_response(StatusCode::OK, 3600, &rotated_keys)),
        ]);
        let client = GoogleJwksClient::with_mock_transport(transport.clone());

        assert_eq!(
            client
                .key_for("old-key")
                .await
                .expect("old key should load")
                .n,
            "old-modulus"
        );
        assert_eq!(
            client.key_for("attacker-key").await,
            Err(AuthError::InvalidToken)
        );
        assert_eq!(
            client
                .key_for("new-key")
                .await
                .expect("rotated key should refresh")
                .n,
            "new-modulus"
        );
        assert!(client.key_for("new-key").await.is_ok());
        assert_eq!(transport.fetch_count(), 3);
    }

    #[tokio::test]
    async fn jwks_fetch_failures_remain_dependency_errors_during_cooldown() {
        let transport = MockGoogleJwksTransport::new(vec![Ok(jwks_response(
            StatusCode::SERVICE_UNAVAILABLE,
            3600,
            &json!([]),
        ))]);
        let client = GoogleJwksClient::with_mock_transport(transport.clone());

        assert_eq!(
            client.key_for("rotated-key").await,
            Err(AuthError::CertificateFetch)
        );
        assert_eq!(
            client.key_for("different-key").await,
            Err(AuthError::CertificateFetch)
        );
        assert_eq!(transport.fetch_count(), 1);
    }

    #[test]
    fn identity_lookup_selects_only_the_requested_user() {
        let response = IdentityLookupResponse {
            users: vec![
                IdentityLookupUser {
                    local_id: "other-user".to_owned(),
                    disabled: false,
                    valid_since: Some("100".to_owned()),
                },
                IdentityLookupUser {
                    local_id: "user-123".to_owned(),
                    disabled: true,
                    valid_since: Some("200".to_owned()),
                },
            ],
        };

        let user = response
            .user("user-123")
            .expect("requested user should exist");
        assert_eq!(user.local_id, "user-123");
        assert!(user.disabled);
        assert!(matches!(
            IdentityLookupResponse { users: vec![] }.user("user-123"),
            Err(AuthError::InvalidToken)
        ));
    }

    #[test]
    fn jwks_cache_ttl_uses_max_age_and_rejects_invalid_directives() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120, must-revalidate"),
        );
        assert_eq!(cache_ttl(&headers), Some(Duration::from_secs(120)));

        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=invalid"),
        );
        assert_eq!(cache_ttl(&headers), None);
    }

    #[test]
    fn external_auth_responses_must_have_success_statuses() {
        assert_eq!(
            require_successful_response(StatusCode::OK, AuthError::CertificateFetch),
            Ok(())
        );
        assert_eq!(
            require_successful_response(StatusCode::NO_CONTENT, AuthError::ServiceUnavailable),
            Ok(())
        );

        for status in [
            StatusCode::NOT_MODIFIED,
            StatusCode::UNAUTHORIZED,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert_eq!(
                require_successful_response(status, AuthError::CertificateFetch),
                Err(AuthError::CertificateFetch),
                "unexpected result for {status}"
            );
        }
    }

    #[test]
    fn jwt_error_mapping_preserves_expiration_as_a_distinct_failure() {
        assert_eq!(
            map_jwt_error(JwtError::from(JwtErrorKind::ExpiredSignature)),
            AuthError::TokenExpired
        );
        assert_eq!(
            map_jwt_error(JwtError::from(JwtErrorKind::InvalidSignature)),
            AuthError::InvalidToken
        );
    }

    #[test]
    fn auth_log_categories_are_stable_and_non_sensitive() {
        for (error, category) in [
            (AuthError::MissingAuthorization, "no_token"),
            (AuthError::InvalidAuthorization, "invalid_header"),
            (AuthError::InvalidToken, "invalid_token"),
            (AuthError::TokenExpired, "token_expired"),
            (AuthError::TokenRevoked, "token_revoked"),
            (AuthError::UserDisabled, "user_disabled"),
            (AuthError::CertificateFetch, "certificate_fetch_failed"),
            (AuthError::ServiceUnavailable, "service_unavailable"),
        ] {
            assert_eq!(categorize_auth_error(&error), category);
        }

        for error in [
            AuthError::MissingAuthorization,
            AuthError::InvalidAuthorization,
            AuthError::InvalidToken,
            AuthError::TokenExpired,
            AuthError::TokenRevoked,
            AuthError::UserDisabled,
        ] {
            assert!(!auth_error_is_dependency_failure(&error));
        }
        assert!(auth_error_is_dependency_failure(
            &AuthError::CertificateFetch
        ));
        assert!(auth_error_is_dependency_failure(
            &AuthError::ServiceUnavailable
        ));
    }

    #[tokio::test]
    async fn emulator_verifier_accepts_unsigned_tokens() {
        let verifier = EmulatorAuthVerifier {
            project_id: "demo-test-project".to_owned(),
        };
        let now = super::unix_timestamp_now();

        let token = unsigned_token(
            json!({
                "alg": "RS256",
                "typ": "JWT"
            }),
            json!({
                "sub": "user-123",
                "email": "test@example.com",
                "email_verified": true,
                "aud": "demo-test-project",
                "iss": expected_issuer("demo-test-project"),
                "iat": now - 300,
                "auth_time": now - 300,
                "exp": now + 3600
            }),
        );

        let user = verifier.verify(&token).await.expect("token should verify");
        assert_eq!(
            user,
            FirebaseUser::new("user-123", "test@example.com", true)
        );
    }

    #[tokio::test]
    async fn emulator_verifier_rejects_expired_tokens() {
        let verifier = EmulatorAuthVerifier {
            project_id: "demo-test-project".to_owned(),
        };
        let now = super::unix_timestamp_now();

        let token = unsigned_token(
            json!({
                "alg": "RS256",
                "typ": "JWT"
            }),
            json!({
                "sub": "user-123",
                "aud": "demo-test-project",
                "iss": expected_issuer("demo-test-project"),
                "iat": now - 3600,
                "auth_time": now - 3600,
                "exp": now - 120
            }),
        );

        let error = verifier
            .verify(&token)
            .await
            .expect_err("token should fail");
        assert_eq!(error, AuthError::TokenExpired);
    }

    #[tokio::test]
    async fn emulator_verifier_rejects_multi_valued_audience() {
        let verifier = EmulatorAuthVerifier {
            project_id: "demo-test-project".to_owned(),
        };
        let now = super::unix_timestamp_now();

        let token = unsigned_token(
            json!({
                "alg": "RS256",
                "typ": "JWT"
            }),
            json!({
                "sub": "user-123",
                "aud": ["other-project", "demo-test-project"],
                "iss": expected_issuer("demo-test-project"),
                "iat": now - 300,
                "auth_time": now - 300,
                "exp": now + 3600
            }),
        );

        let error = verifier
            .verify(&token)
            .await
            .expect_err("array-valued Firebase audience should fail");
        assert_eq!(error, AuthError::InvalidToken);
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "the test helper consumes inline JSON fixtures"
    )]
    fn unsigned_token(header: serde_json::Value, claims: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&header).expect("header should serialize to JSON"));
        let claims = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).expect("claims should serialize to JSON"));
        format!("{header}.{claims}.")
    }

    fn valid_claims(now: u64) -> FirebaseClaims {
        FirebaseClaims {
            sub: "user-123".to_owned(),
            email: Some("test@example.com".to_owned()),
            email_verified: true,
            exp: Some(now + 3600),
            iat: Some(now),
            auth_time: Some(now),
            iss: Some(expected_issuer("demo-test-project")),
            aud: Some("demo-test-project".to_owned()),
        }
    }

    fn test_config() -> AppConfig {
        AppConfig {
            port: 8080,
            firebase_project_id: "demo-test-project".to_owned(),
            app_environment: crate::config::AppEnvironment::Development,
            github_token: None,
            google_application_credentials: None,
            firebase_auth_emulator_host: None,
            firestore_emulator_host: None,
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        }
    }
}
