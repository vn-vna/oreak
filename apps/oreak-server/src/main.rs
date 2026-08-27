use std::{env, error::Error, net::SocketAddr};

use jsonrpsee::server::ServerHandle;
use oreak_server::build_application;
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:3000";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let address = env::var("OREAK_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN_ADDRESS.to_owned());
    let address: SocketAddr = address.parse()?;
    let listener = TcpListener::bind(address).await?;
    let application = build_application();
    let rpc_handle = application.rpc_handle();

    warn!("MVP application storage is in-memory and non-production; all data is lost on restart");
    info!(address = %listener.local_addr()?, "oreak-server listening");
    axum::serve(listener, application.into_router())
        .with_graceful_shutdown(shutdown_signal(rpc_handle))
        .await?;
    Ok(())
}

async fn shutdown_signal(rpc_handle: ServerHandle) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::error!(%error, "failed to listen for Ctrl+C");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for Ctrl+C");
    }

    let _ = rpc_handle.stop();
    info!("shutdown requested");
}
