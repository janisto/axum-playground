use thiserror::Error;

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("invalid PORT value: {0}")]
    InvalidPort(String),
    #[error("failed to initialize authentication: {0}")]
    AuthInitialization(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to initialize tracing: {0}")]
    Tracing(#[from] tracing_subscriber::util::TryInitError),
}
