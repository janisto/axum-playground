use std::fmt;

use crate::{
    auth::{AuthVerifier, MockAuthVerifier},
    config::AppConfig,
    error::StartupError,
    services::github::{GitHubService, MockGitHubService},
    services::profile::{MockProfileService, ProfileService},
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
        let github_service = if config.app_environment == "test" {
            GitHubService::mock(MockGitHubService::demo())
        } else {
            GitHubService::http(config.github_token.clone())
        };

        let auth_verifier = if config.app_environment == "test" {
            AuthVerifier::mock(MockAuthVerifier::test_user())
        } else {
            AuthVerifier::from_config(&config)?
        };

        let profile_service = if config.app_environment == "test" {
            ProfileService::mock(MockProfileService::default())
        } else {
            ProfileService::firestore(&config)
        };

        Ok(Self::with_services(
            config,
            github_service,
            auth_verifier,
            profile_service,
        ))
    }

    pub fn with_github_service(
        config: AppConfig,
        github_service: GitHubService,
    ) -> Result<Self, StartupError> {
        let auth_verifier = if config.app_environment == "test" {
            AuthVerifier::mock(MockAuthVerifier::test_user())
        } else {
            AuthVerifier::from_config(&config)?
        };

        let profile_service = if config.app_environment == "test" {
            ProfileService::mock(MockProfileService::default())
        } else {
            ProfileService::firestore(&config)
        };

        Ok(Self::with_services(
            config,
            github_service,
            auth_verifier,
            profile_service,
        ))
    }

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
