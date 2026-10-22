pub async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(
                    event.name = "service.shutdown.signal_registration_failed",
                    component = "platform_runtime",
                    signal = "SIGTERM",
                    error_code = %error
                );
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::error!(
                    event.name = "service.shutdown.signal_failed",
                    component = "platform_runtime",
                    signal = "SIGINT",
                    error_code = %error
                );
            }
        }
        _ = terminate => {
            tracing::info!(
                event.name = "service.shutdown.signal_received",
                component = "platform_runtime",
                signal = "SIGTERM"
            );
        }
    }

    tracing::info!(
        event.name = "service.shutdown.started",
        service = "payvia-platform"
    );
}
