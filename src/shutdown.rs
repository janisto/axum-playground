use std::future::Future;

#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

#[derive(Debug, Eq, PartialEq)]
enum ShutdownCause {
    CtrlC,
    Terminate,
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            let _ = stream.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let _ = wait_for_shutdown(ctrl_c, terminate).await;
}

async fn wait_for_shutdown(
    ctrl_c: impl Future<Output = ()>,
    terminate: impl Future<Output = ()>,
) -> ShutdownCause {
    tokio::pin!(ctrl_c);
    tokio::pin!(terminate);

    tokio::select! {
        () = &mut ctrl_c => ShutdownCause::CtrlC,
        () = &mut terminate => ShutdownCause::Terminate,
    }
}

#[cfg(test)]
mod tests {
    use std::future::{pending, ready};

    use super::{ShutdownCause, wait_for_shutdown};

    #[tokio::test]
    async fn ctrl_c_completes_shutdown() {
        let cause = wait_for_shutdown(ready(()), pending()).await;

        assert_eq!(cause, ShutdownCause::CtrlC);
    }

    #[tokio::test]
    async fn terminate_completes_shutdown() {
        let cause = wait_for_shutdown(pending(), ready(())).await;

        assert_eq!(cause, ShutdownCause::Terminate);
    }
}
