use axum_observability::{FieldConvention, ObservabilityConfig};
use tracing_subscriber::{EnvFilter, prelude::*};

use crate::error::StartupError;

pub(crate) fn observability_config() -> ObservabilityConfig {
    ObservabilityConfig::default()
        .with_field_convention(FieldConvention::Gcp)
        .with_raw_path(true)
}

pub fn init_tracing(app_environment: &str) -> Result<(), StartupError> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter(app_environment)));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(observability_config().json_layer(std::io::stdout))
        .try_init()?;

    Ok(())
}

fn default_filter(app_environment: &str) -> &'static str {
    match app_environment {
        "production" => "info,axum_playground=info",
        _ => "debug,axum_playground=debug",
    }
}

#[cfg(test)]
mod tests {
    use super::{default_filter, observability_config};

    #[test]
    fn production_uses_less_verbose_default_filter() {
        assert_eq!(default_filter("production"), "info,axum_playground=info");
        assert_eq!(default_filter("development"), "debug,axum_playground=debug");
        assert_eq!(default_filter("test"), "debug,axum_playground=debug");
    }

    #[test]
    fn observability_uses_gcp_fields_and_captures_raw_paths() {
        let debug = format!("{:?}", observability_config());

        assert!(debug.contains("field_convention: Gcp"));
        assert!(debug.contains("raw_path: true"));
    }
}
