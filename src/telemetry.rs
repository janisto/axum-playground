use axum_observability::{FieldConvention, ObservabilityConfig, TraceContextLevel};
use tracing_subscriber::{EnvFilter, prelude::*};

use crate::{config::AppEnvironment, error::StartupError};

/// Returns the finalized configuration shared by the JSON formatter and HTTP middleware.
pub fn observability_config() -> ObservabilityConfig {
    ObservabilityConfig::default()
        .with_field_convention(FieldConvention::Gcp)
        .with_trace_context_level(TraceContextLevel::Level1)
        .with_request_id_validator(valid_portable_request_id)
}

fn valid_portable_request_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

pub fn init_tracing(app_environment: AppEnvironment) -> Result<(), StartupError> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter(app_environment)));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(observability_config().json_layer(std::io::stdout))
        .try_init()?;

    Ok(())
}

fn default_filter(app_environment: AppEnvironment) -> &'static str {
    match app_environment {
        AppEnvironment::Production => "info,axum_playground=info",
        AppEnvironment::Development | AppEnvironment::Test => "debug,axum_playground=debug",
    }
}

#[cfg(test)]
mod tests {
    use axum_observability::TraceContextLevel;

    use crate::config::AppEnvironment;

    use super::{default_filter, observability_config, valid_portable_request_id};

    #[test]
    fn production_uses_less_verbose_default_filter() {
        assert_eq!(
            default_filter(AppEnvironment::Production),
            "info,axum_playground=info"
        );
        assert_eq!(
            default_filter(AppEnvironment::Development),
            "debug,axum_playground=debug"
        );
        assert_eq!(
            default_filter(AppEnvironment::Test),
            "debug,axum_playground=debug"
        );
    }

    #[test]
    fn observability_uses_level_one_trace_context() {
        assert_eq!(
            observability_config().trace_context_level(),
            TraceContextLevel::Level1
        );
    }

    #[test]
    fn request_id_validator_uses_the_portable_grammar() {
        assert!(valid_portable_request_id("A"));
        assert!(valid_portable_request_id(&format!(
            "A{}",
            "._:-".repeat(31)
        )));
        for invalid in ["", "-bad", "bad id", "bad/id", "bad,id", "é"] {
            assert!(!valid_portable_request_id(invalid));
        }
        assert!(!valid_portable_request_id(&"x".repeat(129)));
    }
}
