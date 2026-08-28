use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use command_group::{AsyncCommandGroup, AsyncGroupChild};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{sleep, timeout},
};

use crate::{
    commands::CommandSpec,
    config::{connect_address, public_address},
};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub async fn supervise(
    server_spec: &CommandSpec,
    frontend_spec: &CommandSpec,
    server_address: SocketAddr,
    frontend_address: SocketAddr,
) -> Result<()> {
    let mut server = spawn_group("server", server_spec)?;
    if let Err(error) = wait_for_health(&mut server, server_address).await {
        stop_group(&mut server).await;
        return Err(error);
    }

    let mut frontend = match spawn_group("frontend", frontend_spec) {
        Ok(child) => child,
        Err(error) => {
            stop_group(&mut server).await;
            return Err(error);
        }
    };
    if let Err(error) = wait_for_http(&mut frontend, frontend_address, "/", "frontend").await {
        stop_group(&mut frontend).await;
        stop_group(&mut server).await;
        return Err(error);
    }
    println!(
        "Oreak development URL: http://{}",
        public_address(frontend_address)
    );
    println!("Press Ctrl+C to stop both processes.");

    let outcome: Result<Option<(&str, std::process::ExitStatus)>> = loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                break signal
                    .context("failed to listen for Ctrl+C")
                    .map(|()| None);
            }
            _ = sleep(POLL_INTERVAL) => {
                match server.try_wait().context("failed to query server process") {
                    Ok(Some(status)) => break Ok(Some(("server", status))),
                    Ok(None) => {}
                    Err(error) => break Err(error),
                }
                match frontend.try_wait().context("failed to query frontend process") {
                    Ok(Some(status)) => break Ok(Some(("frontend", status))),
                    Ok(None) => {}
                    Err(error) => break Err(error),
                }
            }
        }
    };

    stop_group(&mut frontend).await;
    stop_group(&mut server).await;
    if let Some((name, status)) = outcome? {
        bail!("{name} exited unexpectedly with {status}");
    }
    Ok(())
}

fn spawn_group(name: &str, spec: &CommandSpec) -> Result<AsyncGroupChild> {
    println!("+ {}", spec.display());
    spec.command()
        .group_spawn()
        .with_context(|| format!("failed to start {name}"))
}

async fn wait_for_health(child: &mut AsyncGroupChild, address: SocketAddr) -> Result<()> {
    wait_for_http(child, address, "/health", "server").await
}

async fn wait_for_http(
    child: &mut AsyncGroupChild,
    address: SocketAddr,
    path: &str,
    name: &str,
) -> Result<()> {
    let started = Instant::now();
    let address = connect_address(address);
    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to query {name} process"))?
        {
            bail!("{name} exited before becoming ready with {status}");
        }
        if http_check(address, path).await {
            println!("{name} readiness check passed at http://{address}{path}");
            return Ok(());
        }
        if started.elapsed() >= HEALTH_TIMEOUT {
            bail!(
                "{name} did not become ready within {} seconds",
                HEALTH_TIMEOUT.as_secs()
            );
        }
        sleep(POLL_INTERVAL).await;
    }
}

async fn http_check(address: SocketAddr, path: &str) -> bool {
    timeout(Duration::from_secs(2), async move {
        let mut stream = TcpStream::connect(address).await?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;
        let mut response = [0_u8; 64];
        let read = stream.read(&mut response).await?;
        Ok::<bool, std::io::Error>(
            response[..read].starts_with(b"HTTP/1.1 200")
                || response[..read].starts_with(b"HTTP/1.0 200"),
        )
    })
    .await
    .is_ok_and(|result| result.unwrap_or(false))
}

async fn stop_group(child: &mut AsyncGroupChild) {
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(error) = child.kill().await {
                eprintln!("warning: failed to terminate child process group: {error}");
            }
        }
        Err(error) => eprintln!("warning: failed to query child process group: {error}"),
    }
}
