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
        if let Some(host) = config.firestore_emulator_host.as_deref()
            && !config.emulator_host_is_loopback(host)
        {
            return Err(StartupError::UnsafeEmulatorHost {
                variable: "FIRESTORE_EMULATOR_HOST",
                host: host.to_string(),
            });
        }

        let github_service = GitHubService::http(config.github_token.clone());
        let auth_verifier = AuthVerifier::from_config(&config)?;
        let profile_service = ProfileService::firestore(&config);

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

#[cfg(test)]
mod tests {
    use crate::{config::AppConfig, error::StartupError};

    use super::AppState;

    #[test]
    fn runtime_state_rejects_non_loopback_firestore_emulator() {
        let config = AppConfig {
            port: 8080,
            firebase_project_id: "project".to_string(),
            app_environment: "development".to_string(),
            github_token: None,
            google_application_credentials: None,
            firebase_auth_emulator_host: None,
            firestore_emulator_host: Some("firestore.example.com:8080".to_string()),
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
}
