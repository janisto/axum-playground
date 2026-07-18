use std::fmt;

use crate::{
    auth::AuthVerifier, config::AppConfig, error::StartupError, services::github::GitHubService,
    services::profile::ProfileService,
};

pub struct AppState {
    pub config: AppConfig,
    pub github_service: GitHubService,
    pub auth_verifier: AuthVerifier,
    pub profile_service: ProfileService,
}

impl fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppState")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub fn new(config: AppConfig) -> Result<Self, StartupError> {
        let profile_service = ProfileService::firestore(&config)?;
        let github_service = GitHubService::http(config.github_token.clone());
        let auth_verifier = AuthVerifier::from_config(&config)?;

        Ok(Self::with_services(
            config,
            github_service,
            auth_verifier,
            profile_service,
        ))
    }

    #[must_use]
    pub fn with_services(
        config: AppConfig,
        github_service: GitHubService,
        auth_verifier: AuthVerifier,
        profile_service: ProfileService,
    ) -> Self {
        Self {
            config,
            github_service,
            auth_verifier,
            profile_service,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        auth::{AuthVerifier, MockAuthVerifier},
        config::{AppConfig, AppEnvironment},
        error::StartupError,
        services::{
            github::{GitHubService, MockGitHubService},
            profile::{MockProfileService, ProfileService},
        },
    };

    use super::AppState;

    #[test]
    fn runtime_state_rejects_non_loopback_firestore_emulator() {
        let config = AppConfig {
            port: 8080,
            firebase_project_id: "project".to_owned(),
            app_environment: AppEnvironment::Development,
            github_token: None,
            google_application_credentials: None,
            firebase_auth_emulator_host: None,
            firestore_emulator_host: Some("firestore.example.com:8080".to_owned()),
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        };

        assert!(matches!(
            AppState::new(config),
            Err(StartupError::UnsafeEmulatorHost {
                variable: "FIRESTORE_EMULATOR_HOST",
                ..
            })
        ));
    }

    #[test]
    fn debug_output_keeps_service_state_opaque_and_config_secrets_redacted() {
        let state = AppState::with_services(
            AppConfig {
                port: 8080,
                firebase_project_id: "project".to_owned(),
                app_environment: AppEnvironment::Test,
                github_token: Some("secret-token".to_owned()),
                google_application_credentials: Some("/secret/credentials.json".to_owned()),
                firebase_auth_emulator_host: None,
                firestore_emulator_host: None,
                google_cloud_project: None,
                gcp_project: None,
                gcloud_project: None,
                project_id: None,
            },
            GitHubService::mock(MockGitHubService::demo()),
            AuthVerifier::mock(MockAuthVerifier::test_user()),
            ProfileService::mock(MockProfileService::default()),
        );

        let output = format!("{state:?}");
        assert!(output.starts_with("AppState { config: AppConfig"));
        assert!(!output.contains("secret-token"));
        assert!(!output.contains("/secret/credentials.json"));
        assert!(!output.contains("github_service"));
        assert!(!output.contains("profile_service"));
    }
}
