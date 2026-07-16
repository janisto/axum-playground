use std::{env, fmt};

use crate::error::StartupError;

#[derive(Clone, Eq, PartialEq)]
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

impl fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field("port", &self.port)
            .field("firebase_project_id", &self.firebase_project_id)
            .field("app_environment", &self.app_environment)
            .field(
                "github_token",
                &self.github_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "google_application_credentials",
                &self
                    .google_application_credentials
                    .as_ref()
                    .map(|_| "[REDACTED]"),
            )
            .field(
                "firebase_auth_emulator_host",
                &self.firebase_auth_emulator_host,
            )
            .field("firestore_emulator_host", &self.firestore_emulator_host)
            .field("google_cloud_project", &self.google_cloud_project)
            .field("gcp_project", &self.gcp_project)
            .field("gcloud_project", &self.gcloud_project)
            .field("project_id", &self.project_id)
            .finish()
    }
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

    pub fn allows_local_emulators(&self) -> bool {
        matches!(self.app_environment.as_str(), "development" | "test")
    }

    pub fn emulator_host_is_loopback(&self, host: &str) -> bool {
        if !self.allows_local_emulators() {
            return false;
        }

        let Some((hostname, port)) = host.rsplit_once(':') else {
            return false;
        };
        matches!(hostname, "localhost" | "127.0.0.1" | "[::1]")
            && port.parse::<u16>().is_ok_and(|port| port > 0)
    }
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn debug_output_redacts_secret_bearing_values() {
        let config = AppConfig {
            port: 8080,
            firebase_project_id: "project".to_string(),
            app_environment: "development".to_string(),
            github_token: Some("secret-token".to_string()),
            google_application_credentials: Some("/secret/credentials.json".to_string()),
            firebase_auth_emulator_host: None,
            firestore_emulator_host: None,
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        };

        let output = format!("{config:?}");
        assert!(!output.contains("secret-token"));
        assert!(!output.contains("/secret/credentials.json"));
        assert_eq!(output.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn emulator_hosts_are_limited_to_local_environments_and_loopback() {
        let mut config = AppConfig {
            port: 8080,
            firebase_project_id: "project".to_string(),
            app_environment: "development".to_string(),
            github_token: None,
            google_application_credentials: None,
            firebase_auth_emulator_host: None,
            firestore_emulator_host: None,
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        };

        assert!(config.emulator_host_is_loopback("127.0.0.1:9099"));
        assert!(config.emulator_host_is_loopback("[::1]:9099"));
        assert!(!config.emulator_host_is_loopback("emulator.example.com:9099"));
        assert!(!config.emulator_host_is_loopback("127.0.0.1"));

        config.app_environment = "production".to_string();
        assert!(!config.emulator_host_is_loopback("127.0.0.1:9099"));
    }
}
