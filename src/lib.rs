//! Teatro application implementation.
//!
//! Teatro is an application crate, not a semver-stable Rust SDK. Public modules exist
//! only where the binary and black-box integration tests need an application boundary;
//! the supported external contract is the documented HTTP API.

pub mod api;
pub mod cli;
pub mod config;
pub mod domain;
pub mod error;
pub mod logging;
pub mod ops;
pub mod repositories;
pub mod services;
pub mod state;
pub mod storage;

use std::net::SocketAddr;

use tokio::net::TcpListener;

use crate::{
    config::AppConfig, error::AppError, services::lan_discovery::LanDiscovery, state::AppState,
};

/// Initialize Teatro from environment configuration and serve until shutdown.
pub async fn run() -> Result<(), AppError> {
    let config = AppConfig::from_env()?;
    init_tracing(&config);

    let state = AppState::initialize(config).await?;
    serve(state).await
}

/// Serve an initialized application state until a shutdown signal is received.
pub async fn serve(state: AppState) -> Result<(), AppError> {
    let bind_addr = state.config().bind_addr;
    let listener = TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    let discovery = match LanDiscovery::start(
        &state.config().lan_discovery,
        &state.config().data_dir,
        state.file_store(),
        local_addr,
    )
    .await
    {
        Ok(discovery) => discovery,
        Err(error) => {
            tracing::warn!(
                ?error,
                "LAN discovery failed to start; HTTP remains available"
            );
            None
        }
    };
    let app = api::router(state);

    tracing::info!(%local_addr, "teatro server listening");

    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;

    if let Some(discovery) = discovery {
        discovery.shutdown().await;
    }
    result?;
    Ok(())
}

/// Install the tracing subscriber configured for this process.
pub fn init_tracing(config: &AppConfig) {
    logging::init(config);
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(?error, "failed to install ctrl-c signal handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(?error, "failed to install terminate signal handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
