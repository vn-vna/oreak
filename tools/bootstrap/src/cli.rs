use std::{net::SocketAddr, path::PathBuf};

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "bootstrap", about = "Oreak development and CI bootstrap")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check prerequisites and create bootstrap.local.toml.
    Init(InitArgs),
    /// Run oreak-server from the workspace root.
    Server(ServerArgs),
    /// Run the Trunk frontend development server or watcher.
    Frontend(FrontendArgs),
    /// Run and supervise the server and frontend together.
    Dev(DevArgs),
    /// Build the server and browser PWA sequentially.
    Build(BuildArgs),
    /// Run the same formatting, linting, testing, and build gates as CI.
    Verify,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Use existing settings or defaults without prompting.
    #[arg(long, requires = "yes")]
    pub non_interactive: bool,
    /// Accept prerequisite installations and non-interactive defaults.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub struct ServerArgs {
    /// Build and run the optimized server.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub release: Option<bool>,
    /// Server socket address.
    #[arg(long)]
    pub listen: Option<SocketAddr>,
    /// Directory containing the compiled frontend.
    #[arg(long)]
    pub web_dist: Option<PathBuf>,
    /// Add Secure to session cookies.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub secure_cookies: Option<bool>,
    /// Server tracing filter.
    #[arg(long)]
    pub rust_log: Option<String>,
    /// Acknowledge that a non-loopback server bind is intentional.
    #[arg(long)]
    pub allow_public_bind: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub struct FrontendArgs {
    /// Build the frontend with release optimizations.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub release: Option<bool>,
    /// Frontend socket address.
    #[arg(long)]
    pub listen: Option<SocketAddr>,
    /// Backend socket address used by the generated proxies.
    #[arg(long)]
    pub backend: Option<SocketAddr>,
    /// Open the frontend in a browser.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub open: Option<bool>,
    /// Build continuously without serving files.
    #[arg(long)]
    pub watch: bool,
    /// Acknowledge that a non-loopback frontend bind is intentional.
    #[arg(long)]
    pub allow_public_bind: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub struct DevArgs {
    /// Build both processes with release optimizations.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub release: Option<bool>,
    /// Server socket address.
    #[arg(long)]
    pub server_listen: Option<SocketAddr>,
    /// Frontend socket address.
    #[arg(long)]
    pub frontend_listen: Option<SocketAddr>,
    /// Directory the server should use for compiled frontend assets.
    #[arg(long)]
    pub web_dist: Option<PathBuf>,
    /// Add Secure to session cookies.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub secure_cookies: Option<bool>,
    /// Server tracing filter.
    #[arg(long)]
    pub rust_log: Option<String>,
    /// Open the frontend in a browser.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub open: Option<bool>,
    /// Acknowledge that non-loopback development binds are intentional.
    #[arg(long)]
    pub allow_public_bind: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub struct BuildArgs {
    /// Build optimized server and frontend artifacts.
    #[arg(long)]
    pub release: bool,
}
