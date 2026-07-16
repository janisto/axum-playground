use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder as CredentialsBuilder};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation,
    dangerous::insecure_decode,
    decode, decode_header,
    errors::{Error as JwtError, ErrorKind as JwtErrorKind},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::warn;

use crate::{config::AppConfig, error::StartupError, problem::problem_response, state::AppState};

const CERT_RETRY_AFTER_SECS: &str = "30";
const GOOGLE_JWKS_URL: &str =
    "https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com";
const IDENTITY_LOOKUP_URL: &str = "https://identitytoolkit.googleapis.com/v1/accounts:lookup";
const IDENTITY_TOOLKIT_SCOPE: &str = "https://www.googleapis.com/auth/identitytoolkit";
const DEFAULT_USER_AGENT: &str = "axum-playground/0.1.0";
const DEFAULT_JWKS_TTL: Duration = Duration::from_secs(3600);
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
    client: Client,
    project_id: String,
    jwks_cache: Arc<RwLock<Option<CachedJwks>>>,
    lookup_client: IdentityPlatformLookupClient,
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

#[derive(Clone, Debug)]
struct IdentityPlatformLookupClient {
    client: Client,
    credentials: AccessTokenCredentials,
    project_id: String,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedUser(pub FirebaseUser);

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum AudienceClaim {
    One(String),
    Many(Vec<String>),
}

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
    aud: Option<AudienceClaim>,
}

#[derive(Clone, Debug, Deserialize)]
struct GoogleJwkSet {
    keys: Vec<GoogleJwk>,
}

#[derive(Clone, Debug, Deserialize)]
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
                    host: host.to_string(),
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
                    client: client.clone(),
                    project_id: project_id.clone(),
                    jwks_cache: Arc::new(RwLock::new(None)),
                    lookup_client: IdentityPlatformLookupClient {
                        client,
                        credentials,
                        project_id,
                    },
                },
            ))),
        })
    }

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
    pub fn allow(user: FirebaseUser) -> Self {
        Self { user, error: None }
    }

    pub fn test_user() -> Self {
        Self::allow(FirebaseUser::new("user-123", "test@example.com", true))
    }

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
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::InvalidToken);
        }

        let Some(kid) = header.kid.as_deref() else {
            return Err(AuthError::InvalidToken);
        };

        let jwk = self.jwk_for_kid(kid).await?;
        let key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
            .map_err(|_| AuthError::InvalidToken)?;

        let claims = decode::<FirebaseClaims>(token, &key, &self.validation())
            .map(|data| data.claims)
            .map_err(map_jwt_error)?;

        validate_common_claims(&claims, &self.project_id)?;

        let lookup = self.lookup_client.lookup_user(&claims.sub).await?;
        if lookup.disabled {
            return Err(AuthError::UserDisabled);
        }

        if token_is_revoked(lookup.valid_since_epoch(), claims.auth_time) {
            return Err(AuthError::TokenRevoked);
        }

        Ok(claims.into_user())
    }

    fn validation(&self) -> Validation {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = JWT_LEEWAY_SECS;
        validation.set_audience(&[self.project_id.as_str()]);
        validation.set_issuer(&[expected_issuer(&self.project_id)]);
        validation.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);
        validation
    }

    async fn jwk_for_kid(&self, kid: &str) -> Result<GoogleJwk, AuthError> {
        if let Some(cached) = self.cached_key(kid).await {
            return Ok(cached);
        }

        let response = self
            .client
            .get(GOOGLE_JWKS_URL)
            .send()
            .await
            .map_err(|_| AuthError::CertificateFetch)?;

        if !response.status().is_success() {
            return Err(AuthError::CertificateFetch);
        }

        let ttl = cache_ttl(response.headers()).unwrap_or(DEFAULT_JWKS_TTL);
        let body = response
            .bytes()
            .await
            .map_err(|_| AuthError::CertificateFetch)?;
        let payload: GoogleJwkSet =
            serde_json::from_slice(&body).map_err(|_| AuthError::CertificateFetch)?;

        let keys = payload
            .keys
            .into_iter()
            .map(|key| (key.kid.clone(), key))
            .collect::<HashMap<_, _>>();

        let key = keys.get(kid).cloned().ok_or(AuthError::InvalidToken)?;
        let expires_at = Instant::now() + ttl;

        let mut cache = self.jwks_cache.write().await;
        *cache = Some(CachedJwks { keys, expires_at });

        Ok(key)
    }

    async fn cached_key(&self, kid: &str) -> Option<GoogleJwk> {
        let cache = self.jwks_cache.read().await;
        let cached = cache.as_ref()?;
        if Instant::now() >= cached.expires_at {
            return None;
        }

        cached.keys.get(kid).cloned()
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
                local_id: vec![uid.to_string()],
                target_project_id: self.project_id.clone(),
            })
            .send()
            .await
            .map_err(|_| AuthError::ServiceUnavailable)?;

        if !response.status().is_success() {
            return Err(AuthError::ServiceUnavailable);
        }

        let payload: IdentityLookupResponse = response
            .json()
            .await
            .map_err(|_| AuthError::ServiceUnavailable)?;

        payload
            .users
            .into_iter()
            .find(|user| user.local_id == uid)
            .ok_or(AuthError::InvalidToken)
    }
}

impl IdentityLookupUser {
    fn valid_since_epoch(&self) -> Option<u64> {
        self.valid_since.as_deref()?.parse().ok()
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

impl AudienceClaim {
    fn contains(&self, value: &str) -> bool {
        match self {
            AudienceClaim::One(current) => current == value,
            AudienceClaim::Many(current) => current.iter().any(|entry| entry == value),
        }
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

    Ok(parts[1].to_string())
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
            warn!(reason = %categorize_auth_error(&error), "auth failed: invalid authorization header");
            unauthorized_response(&parts.headers, "missing or invalid authorization header")
        })?;

        let user = state.auth_verifier.verify(&token).await.map_err(|error| {
            warn!(reason = %categorize_auth_error(&error), "auth failed: token verification failed");
            map_auth_error(&parts.headers, error)
        })?;

        parts.extensions.insert(user.clone());
        Ok(Self(user))
    }
}

impl IntoResponse for AuthenticatedUser {
    fn into_response(self) -> Response {
        StatusCode::OK.into_response()
    }
}

fn validate_common_claims(claims: &FirebaseClaims, project_id: &str) -> Result<(), AuthError> {
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

    let now = unix_timestamp_now();
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
    if !audience.contains(project_id) {
        return Err(AuthError::InvalidToken);
    }

    Ok(())
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

fn token_is_revoked(valid_since: Option<u64>, auth_time: Option<u64>) -> bool {
    matches!((valid_since, auth_time), (Some(valid_since), Some(auth_time)) if auth_time < valid_since)
}

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
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;

    use super::{
        AuthError, AuthVerifier, EmulatorAuthVerifier, FirebaseUser, expected_issuer,
        extract_bearer_token, token_is_revoked,
    };
    use crate::{config::AppConfig, error::StartupError};

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
        config.firebase_auth_emulator_host = Some("emulator.example.com:9099".to_string());
        assert!(matches!(
            AuthVerifier::from_config(&config),
            Err(StartupError::UnsafeEmulatorHost { .. })
        ));

        config.firebase_auth_emulator_host = Some("127.0.0.1:9099".to_string());
        config.app_environment = "production".to_string();
        assert!(matches!(
            AuthVerifier::from_config(&config),
            Err(StartupError::UnsafeEmulatorHost { .. })
        ));

        config.app_environment = "development".to_string();
        assert!(AuthVerifier::from_config(&config).is_ok());
    }

    #[test]
    fn revocation_uses_auth_time_instead_of_token_issue_time() {
        assert!(token_is_revoked(Some(200), Some(100)));
        assert!(!token_is_revoked(Some(200), Some(200)));
        assert!(!token_is_revoked(Some(200), None));
    }

    #[tokio::test]
    async fn emulator_verifier_accepts_unsigned_tokens() {
        let verifier = EmulatorAuthVerifier {
            project_id: "demo-test-project".to_string(),
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
            project_id: "demo-test-project".to_string(),
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

    fn unsigned_token(header: serde_json::Value, claims: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&header).expect("header should serialize to JSON"));
        let claims = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).expect("claims should serialize to JSON"));
        format!("{header}.{claims}.")
    }

    fn test_config() -> AppConfig {
        AppConfig {
            port: 8080,
            firebase_project_id: "demo-test-project".to_string(),
            app_environment: "development".to_string(),
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
