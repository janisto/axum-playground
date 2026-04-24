use std::env;

use crate::error::StartupError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    pub port: u16,
    pub firebase_project_id: String,
    pub app_environment: String,
    pub github_token: Option<String>,
    pub google_application_credentials: Option<String>,
    pub firebase_auth_emulator_host: Option<String>,
    pub firestore_emulator_host: Option<String>,
    pub google_cloud_project: Option<String>,
    pub gcp_project: Option<String>,
    pub gcloud_project: Option<String>,
    pub project_id: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, StartupError> {
        let port = env::var("PORT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value
                    .parse::<u16>()
                    .map_err(|_| StartupError::InvalidPort(value))
            })
            .transpose()?
            .unwrap_or(8080);

        Ok(Self {
            port,
            firebase_project_id: env::var("FIREBASE_PROJECT_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "demo-test-project".to_string()),
            app_environment: env::var("APP_ENVIRONMENT")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "development".to_string()),
            github_token: optional_env("GITHUB_TOKEN"),
            google_application_credentials: optional_env("GOOGLE_APPLICATION_CREDENTIALS"),
            firebase_auth_emulator_host: optional_env("FIREBASE_AUTH_EMULATOR_HOST"),
            firestore_emulator_host: optional_env("FIRESTORE_EMULATOR_HOST"),
            google_cloud_project: optional_env("GOOGLE_CLOUD_PROJECT"),
            gcp_project: optional_env("GCP_PROJECT"),
            gcloud_project: optional_env("GCLOUD_PROJECT"),
            project_id: optional_env("PROJECT_ID"),
        })
    }

    pub fn resolved_google_project_id(&self) -> Option<&str> {
        self.google_cloud_project
            .as_deref()
            .or(self.gcp_project.as_deref())
            .or(self.gcloud_project.as_deref())
            .or(self.project_id.as_deref())
            .or(Some(self.firebase_project_id.as_str()))
    }
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}
