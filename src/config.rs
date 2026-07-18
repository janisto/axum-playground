use std::{env, fmt, str::FromStr};

use crate::error::StartupError;

#[derive(Clone, Eq, PartialEq)]
pub struct AppConfig {
    pub port: u16,
    pub firebase_project_id: String,
    pub app_environment: AppEnvironment,
    pub github_token: Option<String>,
    pub google_application_credentials: Option<String>,
    pub firebase_auth_emulator_host: Option<String>,
    pub firestore_emulator_host: Option<String>,
    pub google_cloud_project: Option<String>,
    pub gcp_project: Option<String>,
    pub gcloud_project: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AppEnvironment {
    #[default]
    Development,
    Test,
    Production,
}

impl AppEnvironment {
    #[must_use]
    pub const fn allows_local_emulators(self) -> bool {
        matches!(self, Self::Development | Self::Test)
    }
}

impl fmt::Display for AppEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Production => "production",
        })
    }
}

impl FromStr for AppEnvironment {
    type Err = StartupError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "development" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "production" => Ok(Self::Production),
            _ => Err(StartupError::InvalidEnvironment(value.to_owned())),
        }
    }
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
        Self::from_values(|key| env::var(key).ok())
    }

    fn from_values(
        mut value_for: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, StartupError> {
        let port = non_blank(value_for("PORT"))
            .map(|value| {
                value
                    .parse::<u16>()
                    .map_err(|_| StartupError::InvalidPort(value))
            })
            .transpose()?
            .unwrap_or(8080);

        Ok(Self {
            port,
            firebase_project_id: non_blank(value_for("FIREBASE_PROJECT_ID"))
                .unwrap_or_else(|| "demo-test-project".to_owned()),
            app_environment: non_blank(value_for("APP_ENVIRONMENT"))
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or_default(),
            github_token: non_blank(value_for("GITHUB_TOKEN")),
            google_application_credentials: non_blank(value_for("GOOGLE_APPLICATION_CREDENTIALS")),
            firebase_auth_emulator_host: non_blank(value_for("FIREBASE_AUTH_EMULATOR_HOST")),
            firestore_emulator_host: non_blank(value_for("FIRESTORE_EMULATOR_HOST")),
            google_cloud_project: non_blank(value_for("GOOGLE_CLOUD_PROJECT")),
            gcp_project: non_blank(value_for("GCP_PROJECT")),
            gcloud_project: non_blank(value_for("GCLOUD_PROJECT")),
            project_id: non_blank(value_for("PROJECT_ID")),
        })
    }

    #[must_use]
    pub fn resolved_google_project_id(&self) -> &str {
        self.google_cloud_project
            .as_deref()
            .or(self.gcp_project.as_deref())
            .or(self.gcloud_project.as_deref())
            .or(self.project_id.as_deref())
            .unwrap_or(self.firebase_project_id.as_str())
    }

    #[must_use]
    pub fn allows_local_emulators(&self) -> bool {
        self.app_environment.allows_local_emulators()
    }

    #[must_use]
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

fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{AppConfig, AppEnvironment, StartupError};

    #[test]
    fn config_values_apply_defaults_and_ignore_blank_input() {
        let values = HashMap::from([
            ("PORT", "  "),
            ("FIREBASE_PROJECT_ID", ""),
            ("APP_ENVIRONMENT", "\t"),
            ("GITHUB_TOKEN", "\n"),
        ]);

        let config = AppConfig::from_values(|key| values.get(key).map(ToString::to_string))
            .expect("blank values should use defaults");

        assert_eq!(config.port, 8080);
        assert_eq!(config.firebase_project_id, "demo-test-project");
        assert_eq!(config.app_environment, AppEnvironment::Development);
        assert_eq!(config.github_token, None);
    }

    #[test]
    fn config_values_preserve_explicit_settings() {
        let values = HashMap::from([
            ("PORT", "9090"),
            ("FIREBASE_PROJECT_ID", "firebase-project"),
            ("APP_ENVIRONMENT", "production"),
            ("GITHUB_TOKEN", "github-token"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "/credentials.json"),
            ("FIREBASE_AUTH_EMULATOR_HOST", "127.0.0.1:9099"),
            ("FIRESTORE_EMULATOR_HOST", "127.0.0.1:8080"),
            ("GOOGLE_CLOUD_PROJECT", "google-cloud-project"),
            ("GCP_PROJECT", "gcp-project"),
            ("GCLOUD_PROJECT", "gcloud-project"),
            ("PROJECT_ID", "project-id"),
        ]);

        let config = AppConfig::from_values(|key| values.get(key).map(ToString::to_string))
            .expect("explicit values should parse");

        assert_eq!(
            config,
            AppConfig {
                port: 9090,
                firebase_project_id: "firebase-project".to_owned(),
                app_environment: AppEnvironment::Production,
                github_token: Some("github-token".to_owned()),
                google_application_credentials: Some("/credentials.json".to_owned()),
                firebase_auth_emulator_host: Some("127.0.0.1:9099".to_owned()),
                firestore_emulator_host: Some("127.0.0.1:8080".to_owned()),
                google_cloud_project: Some("google-cloud-project".to_owned()),
                gcp_project: Some("gcp-project".to_owned()),
                gcloud_project: Some("gcloud-project".to_owned()),
                project_id: Some("project-id".to_owned()),
            }
        );
    }

    #[test]
    fn config_values_reject_invalid_ports() {
        let error = AppConfig::from_values(|key| (key == "PORT").then(|| "invalid".to_owned()))
            .expect_err("invalid port should fail");

        assert!(matches!(error, StartupError::InvalidPort(value) if value == "invalid"));
    }

    #[test]
    fn config_values_reject_unknown_environments() {
        let error =
            AppConfig::from_values(|key| (key == "APP_ENVIRONMENT").then(|| "staging".to_owned()))
                .expect_err("unknown environment should fail");

        assert!(matches!(
            error,
            StartupError::InvalidEnvironment(value) if value == "staging"
        ));
    }

    #[test]
    fn environment_values_parse_and_display_canonically() {
        for (value, expected) in [
            ("development", AppEnvironment::Development),
            ("test", AppEnvironment::Test),
            ("production", AppEnvironment::Production),
        ] {
            let parsed = value
                .parse::<AppEnvironment>()
                .expect("documented environment should parse");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), value);
        }
    }

    #[test]
    fn debug_output_redacts_secret_bearing_values() {
        let config = AppConfig {
            port: 8080,
            firebase_project_id: "project".to_owned(),
            app_environment: AppEnvironment::Development,
            github_token: Some("secret-token".to_owned()),
            google_application_credentials: Some("/secret/credentials.json".to_owned()),
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
            firebase_project_id: "project".to_owned(),
            app_environment: AppEnvironment::Development,
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
        assert!(!config.emulator_host_is_loopback("127.0.0.1:0"));
        assert!(!config.emulator_host_is_loopback("emulator.example.com:9099"));
        assert!(!config.emulator_host_is_loopback("127.0.0.1"));

        config.app_environment = AppEnvironment::Production;
        assert!(!config.emulator_host_is_loopback("127.0.0.1:9099"));
    }

    #[test]
    fn google_project_resolution_uses_documented_precedence_and_fallback() {
        let mut config = AppConfig {
            port: 8080,
            firebase_project_id: "firebase".to_owned(),
            app_environment: AppEnvironment::Development,
            github_token: None,
            google_application_credentials: None,
            firebase_auth_emulator_host: None,
            firestore_emulator_host: None,
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        };

        assert_eq!(config.resolved_google_project_id(), "firebase");
        config.project_id = Some("project-id".to_owned());
        config.gcloud_project = Some("gcloud".to_owned());
        config.gcp_project = Some("gcp".to_owned());
        config.google_cloud_project = Some("google-cloud".to_owned());
        assert_eq!(config.resolved_google_project_id(), "google-cloud");
    }
}
