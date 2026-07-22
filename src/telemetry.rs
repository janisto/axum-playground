use axum_observability::{FieldConvention, ObservabilityConfig, TraceContextLevel};
use tracing_subscriber::{EnvFilter, prelude::*};

use crate::{config::AppEnvironment, error::StartupError};

/// Returns the finalized configuration shared by the JSON formatter and HTTP middleware.
pub fn observability_config() -> ObservabilityConfig {
    ObservabilityConfig::default()
        .with_field_convention(FieldConvention::Gcp)
        .with_trace_context_level(TraceContextLevel::Level1)
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

    use super::{default_filter, observability_config};

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
}
