use thiserror::Error;

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("invalid PORT value: {0}")]
    InvalidPort(String),
    #[error("invalid APP_ENVIRONMENT value: {0}; expected development, test, or production")]
    InvalidEnvironment(String),
    #[error("Cloud Run requires APP_ENVIRONMENT=production")]
    InvalidCloudRunEnvironment,
    #[error("Cloud Run requires an explicit {0} value")]
    MissingCloudRunVariable(&'static str),
    #[error("failed to initialize authentication credentials")]
    AuthCredentialsInitialization,
    #[error("failed to initialize authentication HTTP client")]
    AuthHttpClientInitialization,
    #[error("unsafe emulator configuration for {variable}: {host}")]
    UnsafeEmulatorHost {
        variable: &'static str,
        host: String,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to initialize tracing: {0}")]
    Tracing(#[from] tracing_subscriber::util::TryInitError),
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::StartupError;

    #[test]
    fn authentication_initialization_errors_are_safe_and_actionable() {
        let credentials = StartupError::AuthCredentialsInitialization;
        assert_eq!(
            credentials.to_string(),
            "failed to initialize authentication credentials"
        );
        assert!(credentials.source().is_none());

        let http_client = StartupError::AuthHttpClientInitialization;
        assert_eq!(
            http_client.to_string(),
            "failed to initialize authentication HTTP client"
        );
        assert!(http_client.source().is_none());
    }
}
