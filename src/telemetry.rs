use axum_observability::{FieldConvention, ObservabilityConfig};
use tracing_subscriber::{EnvFilter, prelude::*};

use crate::error::StartupError;

pub(crate) fn observability_config() -> ObservabilityConfig {
    ObservabilityConfig::default()
        .with_field_convention(FieldConvention::Gcp)
        .with_raw_path(true)
}

pub fn init_tracing(app_environment: &str) -> Result<(), StartupError> {
    let default_filter = match app_environment {
        "production" => "info,axum_playground=info",
        _ => "debug,axum_playground=debug",
    };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(observability_config().json_layer(std::io::stdout))
        .try_init()?;

    Ok(())
}
