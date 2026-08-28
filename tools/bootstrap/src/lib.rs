#![forbid(unsafe_code)]

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

pub mod cli;
pub mod commands;
pub mod config;
mod prerequisites;
mod process;

use cli::{Cli, Command, DevArgs, FrontendArgs, ServerArgs};
use config::LocalConfig;

pub async fn run(cli: Cli) -> Result<()> {
    let root = workspace_root()?;
    match cli.command {
        Command::Init(args) => init(&root, args.non_interactive, args.yes),
        Command::Server(args) => run_server(&root, args).await,
        Command::Frontend(args) => run_frontend(&root, args).await,
        Command::Dev(args) => run_dev(&root, args).await,
        Command::Build(args) => build(&root, args.release).await,
        Command::Verify => verify(&root).await,
    }
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .context("bootstrap package must be located at <workspace>/tools/bootstrap")?;
    let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
    if !root.join("Cargo.toml").is_file() || !root.join("apps/oreak-web/index.html").is_file() {
        bail!(
            "could not locate the Oreak workspace from {}",
            manifest.display()
        );
    }
    Ok(root)
}

fn init(root: &Path, non_interactive: bool, yes: bool) -> Result<()> {
    let rust = prerequisites::ensure_rust()?;
    println!("Rust {rust} satisfies the minimum 1.85.0 requirement.");

    if prerequisites::wasm_target_installed()? {
        println!("Rust target wasm32-unknown-unknown is installed.");
    } else if yes || confirm("Install Rust target wasm32-unknown-unknown?", true)? {
        prerequisites::install_wasm_target()?;
    } else {
        bail!("wasm32-unknown-unknown is required");
    }

    match prerequisites::trunk_version()? {
        Some(version) if version == commands::TRUNK_VERSION => {
            println!("Trunk {version} is installed.");
        }
        installed => {
            let description = installed.as_deref().unwrap_or("not installed");
            if yes
                || confirm(
                    &format!(
                        "Trunk {description}; install required version {}?",
                        commands::TRUNK_VERSION
                    ),
                    true,
                )?
            {
                prerequisites::install_trunk(installed.is_some())?;
            } else {
                bail!("Trunk {} is required", commands::TRUNK_VERSION);
            }
        }
    }
    prerequisites::ensure_wasm_target()?;
    prerequisites::ensure_trunk()?;

    let existing = config::load(root)?;
    let settings = if non_interactive {
        existing.clone().unwrap_or_default()
    } else {
        prompt_config(existing.unwrap_or_default())?
    };
    let changed = config::write_if_changed(root, &settings)?;
    if changed {
        println!("Wrote {}", root.join(config::CONFIG_FILE).display());
    } else {
        println!(
            "{} is already current.",
            root.join(config::CONFIG_FILE).display()
        );
    }
    Ok(())
}

async fn run_server(root: &Path, args: ServerArgs) -> Result<()> {
    prerequisites::ensure_rust()?;
    let local = config::load(root)?;
    let settings = config::resolve_server(&args, local.as_ref(), &config::environment(), root)?;
    config::validate_bind(settings.listen, args.allow_public_bind, "server")?;
    commands::run_checked(&commands::server(root, &settings)).await
}

async fn run_frontend(root: &Path, args: FrontendArgs) -> Result<()> {
    prerequisites::ensure_rust()?;
    prerequisites::ensure_wasm_target()?;
    prerequisites::ensure_trunk()?;
    let local = config::load(root)?;
    let settings = config::resolve_frontend(&args, local.as_ref(), &config::environment())?;
    config::validate_bind(settings.listen, args.allow_public_bind, "frontend")?;
    let generated = commands::write_generated_trunk(root, &settings)?;
    commands::run_checked(&commands::frontend(root, &generated, &settings, args.watch)).await
}

async fn run_dev(root: &Path, args: DevArgs) -> Result<()> {
    prerequisites::ensure_rust()?;
    prerequisites::ensure_wasm_target()?;
    prerequisites::ensure_trunk()?;
    let local = config::load(root)?;
    let env = config::environment();
    let server_args = ServerArgs {
        release: args.release,
        listen: args.server_listen,
        web_dist: args.web_dist,
        secure_cookies: args.secure_cookies,
        rust_log: args.rust_log,
        allow_public_bind: args.allow_public_bind,
    };
    let server = config::resolve_server(&server_args, local.as_ref(), &env, root)?;
    let frontend_args = FrontendArgs {
        release: args.release,
        listen: args.frontend_listen,
        backend: Some(server.listen),
        open: args.open,
        watch: false,
        allow_public_bind: args.allow_public_bind,
    };
    let frontend = config::resolve_frontend(&frontend_args, local.as_ref(), &env)?;
    config::validate_bind(server.listen, args.allow_public_bind, "server")?;
    config::validate_bind(frontend.listen, args.allow_public_bind, "frontend")?;
    if config::addresses_conflict(server.listen, frontend.listen) {
        bail!(
            "server and frontend addresses conflict: {} and {}",
            server.listen,
            frontend.listen
        );
    }
    let generated = commands::write_generated_trunk(root, &frontend)?;
    process::supervise(
        &commands::server(root, &server),
        &commands::frontend(root, &generated, &frontend, false),
        server.listen,
        frontend.listen,
    )
    .await
}

async fn build(root: &Path, release: bool) -> Result<()> {
    prerequisites::ensure_rust()?;
    prerequisites::ensure_wasm_target()?;
    prerequisites::ensure_trunk()?;
    commands::run_checked(&commands::server_build(root, release)).await?;
    commands::run_checked(&commands::trunk_build(root, release)).await
}

async fn verify(root: &Path) -> Result<()> {
    prerequisites::ensure_rust()?;
    prerequisites::ensure_wasm_target()?;
    prerequisites::ensure_trunk()?;
    for command in commands::verification(root) {
        commands::run_checked(&command).await?;
    }
    Ok(())
}

fn prompt_config(mut config: LocalConfig) -> Result<LocalConfig> {
    config.server.listen = prompt_parse("Server listen", config.server.listen)?;
    config.server.web_dist = PathBuf::from(prompt(
        "Server web distribution",
        &config.server.web_dist.display().to_string(),
    )?);
    config.server.rust_log = prompt("Server RUST_LOG", &config.server.rust_log)?;
    config.server.secure_cookies = prompt_bool("Use secure cookies", config.server.secure_cookies)?;
    config.frontend.listen = prompt_parse("Frontend listen", config.frontend.listen)?;
    config.frontend.open = prompt_bool("Open browser", config.frontend.open)?;
    Ok(config)
}

fn prompt(label: &str, default: &str) -> Result<String> {
    print!("{label} [{default}]: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

fn prompt_parse<T>(label: &str, default: T) -> Result<T>
where
    T: std::fmt::Display + std::str::FromStr,
    T::Err: std::fmt::Display,
{
    loop {
        let value = prompt(label, &default.to_string())?;
        match value.parse() {
            Ok(value) => return Ok(value),
            Err(error) => eprintln!("Invalid value: {error}"),
        }
    }
}

fn prompt_bool(label: &str, default: bool) -> Result<bool> {
    loop {
        let value = prompt(label, if default { "true" } else { "false" })?;
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "y" | "on" => return Ok(true),
            "0" | "false" | "no" | "n" | "off" => return Ok(false),
            _ => eprintln!("Enter true or false."),
        }
    }
}

fn confirm(label: &str, default: bool) -> Result<bool> {
    prompt_bool(label, default)
}
