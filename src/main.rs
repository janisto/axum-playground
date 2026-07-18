use std::{net::SocketAddr, sync::Arc};

use tokio::net::TcpListener;

use axum_playground::{
    AppConfig, AppState, build_app, error::StartupError, shutdown::shutdown_signal,
    telemetry::init_tracing,
};

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    let config = AppConfig::from_env()?;
    init_tracing(config.app_environment)?;

    let state = Arc::new(AppState::new(config.clone())?);
    let app = build_app(state);

    let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], config.port))).await?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}
