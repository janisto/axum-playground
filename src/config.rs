use std::{env, fmt, str::FromStr};

use crate::error::StartupError;

#[derive(Clone, Eq, PartialEq)]
pub struct AppConfig {
    pub port: u16,
    pub firebase_project_id: String,
    pub app_environment: AppEnvironment,
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

        let firebase_project_id = non_blank(value_for("FIREBASE_PROJECT_ID"));
        let app_environment = non_blank(value_for("APP_ENVIRONMENT"))
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or_default();
        let is_cloud_run = non_blank(value_for("K_SERVICE")).is_some();

        if is_cloud_run && app_environment != AppEnvironment::Production {
            return Err(StartupError::InvalidCloudRunEnvironment);
        }
        if is_cloud_run && firebase_project_id.is_none() {
            return Err(StartupError::MissingCloudRunVariable("FIREBASE_PROJECT_ID"));
        }

        let config = Self {
            port,
            firebase_project_id: firebase_project_id
                .unwrap_or_else(|| "demo-test-project".to_owned()),
            app_environment,
            google_application_credentials: non_blank(value_for("GOOGLE_APPLICATION_CREDENTIALS")),
            firebase_auth_emulator_host: non_blank(value_for("FIREBASE_AUTH_EMULATOR_HOST")),
            firestore_emulator_host: non_blank(value_for("FIRESTORE_EMULATOR_HOST")),
            google_cloud_project: non_blank(value_for("GOOGLE_CLOUD_PROJECT")),
            gcp_project: non_blank(value_for("GCP_PROJECT")),
            gcloud_project: non_blank(value_for("GCLOUD_PROJECT")),
            project_id: non_blank(value_for("PROJECT_ID")),
        };
        config.validate_emulators()?;
        Ok(config)
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

    pub(crate) fn validate_emulators(&self) -> Result<(), StartupError> {
        let auth = self.firebase_auth_emulator_host.as_deref();
        let firestore = self.firestore_emulator_host.as_deref();
        if auth.is_none() && firestore.is_none() {
            return Ok(());
        }
        for (variable, host) in [
            ("FIREBASE_AUTH_EMULATOR_HOST", auth),
            ("FIRESTORE_EMULATOR_HOST", firestore),
        ] {
            if let Some(host) = host
                && !self.emulator_host_is_loopback(host)
            {
                return Err(StartupError::UnsafeEmulatorHost {
                    variable,
                    host: host.to_owned(),
                });
            }
        }
        if auth.is_none()
            || firestore.is_none()
            || !self.firebase_project_id.starts_with("demo-")
            || self.resolved_google_project_id() != self.firebase_project_id
        {
            return Err(StartupError::InvalidEmulatorConfiguration);
        }
        Ok(())
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
            && port.bytes().all(|byte| byte.is_ascii_digit())
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
        ]);

        let config = AppConfig::from_values(|key| values.get(key).map(ToString::to_string))
            .expect("blank values should use defaults");

        assert_eq!(config.port, 8080);
        assert_eq!(config.firebase_project_id, "demo-test-project");
        assert_eq!(config.app_environment, AppEnvironment::Development);
    }

    #[test]
    fn emulator_configuration_rejects_partial_pairs_and_live_project_selection() {
        for (name, overrides) in [
            ("missing Auth", vec![("FIREBASE_AUTH_EMULATOR_HOST", None)]),
            ("missing Firestore", vec![("FIRESTORE_EMULATOR_HOST", None)]),
            (
                "blank Auth",
                vec![("FIREBASE_AUTH_EMULATOR_HOST", Some(" "))],
            ),
            (
                "real Firebase project",
                vec![("FIREBASE_PROJECT_ID", Some("live-project"))],
            ),
            (
                "Google project override",
                vec![("GOOGLE_CLOUD_PROJECT", Some("live-project"))],
            ),
            (
                "GCP project override",
                vec![("GCP_PROJECT", Some("live-project"))],
            ),
            (
                "gcloud project override",
                vec![("GCLOUD_PROJECT", Some("live-project"))],
            ),
            (
                "generic project override",
                vec![("PROJECT_ID", Some("live-project"))],
            ),
            (
                "different demo project",
                vec![("GOOGLE_CLOUD_PROJECT", Some("demo-other"))],
            ),
            ("production", vec![("APP_ENVIRONMENT", Some("production"))]),
            (
                "remote Auth",
                vec![("FIREBASE_AUTH_EMULATOR_HOST", Some("auth.example.com:7110"))],
            ),
            (
                "remote Firestore",
                vec![("FIRESTORE_EMULATOR_HOST", Some("store.example.com:7130"))],
            ),
            (
                "signed port",
                vec![("FIRESTORE_EMULATOR_HOST", Some("127.0.0.1:+7130"))],
            ),
        ] {
            let mut values = HashMap::from([
                ("APP_ENVIRONMENT", "development"),
                ("FIREBASE_PROJECT_ID", "demo-test-project"),
                ("FIREBASE_AUTH_EMULATOR_HOST", "127.0.0.1:7110"),
                ("FIRESTORE_EMULATOR_HOST", "127.0.0.1:7130"),
            ]);
            for (key, value) in overrides {
                if let Some(value) = value {
                    values.insert(key, value);
                } else {
                    values.remove(key);
                }
            }
            assert!(
                AppConfig::from_values(|key| values.get(key).map(ToString::to_string)).is_err(),
                "unsafe emulator configuration was accepted: {name}"
            );
        }
    }

    #[test]
    fn config_values_preserve_explicit_settings() {
        let values = HashMap::from([
            ("PORT", "9090"),
            ("FIREBASE_PROJECT_ID", "firebase-project"),
            ("APP_ENVIRONMENT", "production"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "/credentials.json"),
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
                google_application_credentials: Some("/credentials.json".to_owned()),
                firebase_auth_emulator_host: None,
                firestore_emulator_host: None,
                google_cloud_project: Some("google-cloud-project".to_owned()),
                gcp_project: Some("gcp-project".to_owned()),
                gcloud_project: Some("gcloud-project".to_owned()),
                project_id: Some("project-id".to_owned()),
            }
        );
    }

    #[test]
    fn emulator_configuration_preserves_paired_hosts_and_the_effective_demo_project() {
        for environment in ["development", "test"] {
            for host in ["localhost:7110", "127.0.0.1:7110", "[::1]:7110"] {
                let values = HashMap::from([
                    ("APP_ENVIRONMENT", environment),
                    ("FIREBASE_AUTH_EMULATOR_HOST", host),
                    ("FIRESTORE_EMULATOR_HOST", "127.0.0.1:7130"),
                    ("GOOGLE_CLOUD_PROJECT", "demo-test-project"),
                    ("GCP_PROJECT", "unused-live-project"),
                ]);
                let config = AppConfig::from_values(|key| values.get(key).map(ToString::to_string))
                    .expect("paired local emulators should use the default demo project");
                assert_eq!(config.firebase_auth_emulator_host.as_deref(), Some(host));
                assert_eq!(
                    config.firestore_emulator_host.as_deref(),
                    Some("127.0.0.1:7130")
                );
                assert_eq!(config.firebase_project_id, "demo-test-project");
                assert_eq!(config.resolved_google_project_id(), "demo-test-project");
            }
        }
    }

    #[test]
    fn cloud_run_requires_explicit_production_identity_configuration() {
        let missing_environment = HashMap::from([
            ("K_SERVICE", "axum-playground"),
            ("FIREBASE_PROJECT_ID", "production-project"),
        ]);
        assert!(matches!(
            AppConfig::from_values(|key| missing_environment.get(key).map(ToString::to_string)),
            Err(StartupError::InvalidCloudRunEnvironment)
        ));

        let missing_project = HashMap::from([
            ("K_SERVICE", "axum-playground"),
            ("APP_ENVIRONMENT", "production"),
        ]);
        assert!(matches!(
            AppConfig::from_values(|key| missing_project.get(key).map(ToString::to_string)),
            Err(StartupError::MissingCloudRunVariable("FIREBASE_PROJECT_ID"))
        ));

        let configured = HashMap::from([
            ("K_SERVICE", "axum-playground"),
            ("APP_ENVIRONMENT", "production"),
            ("FIREBASE_PROJECT_ID", "production-project"),
        ]);
        let config = AppConfig::from_values(|key| configured.get(key).map(ToString::to_string))
            .expect("explicit Cloud Run identity should be accepted");
        assert_eq!(config.app_environment, AppEnvironment::Production);
        assert_eq!(config.firebase_project_id, "production-project");
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
            google_application_credentials: Some("/secret/credentials.json".to_owned()),
            firebase_auth_emulator_host: None,
            firestore_emulator_host: None,
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        };

        let output = format!("{config:?}");
        assert!(!output.contains("/secret/credentials.json"));
        assert_eq!(output.matches("[REDACTED]").count(), 1);
    }

    #[test]
    fn emulator_hosts_are_limited_to_local_environments_and_loopback() {
        let mut config = AppConfig {
            port: 8080,
            firebase_project_id: "project".to_owned(),
            app_environment: AppEnvironment::Development,
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
