use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::error::StartupError;

pub fn init_tracing(app_environment: &str) -> Result<(), StartupError> {
    let default_filter = match app_environment {
        "production" => "info,axum_playground=info",
        _ => "debug,axum_playground=debug",
    };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .json()
                .with_current_span(false)
                .with_span_list(false),
        )
        .try_init()?;

    Ok(())
}
