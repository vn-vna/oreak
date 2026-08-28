use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    bootstrap::run(bootstrap::cli::Cli::parse()).await
}
