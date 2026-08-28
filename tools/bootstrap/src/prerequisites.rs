use std::process::Command;

use anyhow::{Context, Result, bail};
use semver::Version;

use crate::commands::TRUNK_VERSION;

const MINIMUM_RUST: Version = Version::new(1, 85, 0);
const WASM_TARGET: &str = "wasm32-unknown-unknown";

pub fn ensure_rust() -> Result<Version> {
    let output = output("rustc", &["--version"])?;
    let version = parse_tool_version(&output, "rustc")?;
    if version < MINIMUM_RUST {
        bail!("Oreak requires Rust {MINIMUM_RUST} or newer; found {version}");
    }
    Ok(version)
}

pub fn ensure_wasm_target() -> Result<()> {
    let installed = output("rustup", &["target", "list", "--installed"])?;
    if !installed.lines().any(|line| line.trim() == WASM_TARGET) {
        bail!("Rust target {WASM_TARGET} is missing; run `cargo run -p bootstrap -- init`");
    }
    Ok(())
}

pub fn ensure_trunk() -> Result<()> {
    let installed = trunk_version()?;
    if installed.as_deref() != Some(TRUNK_VERSION) {
        let found = installed.unwrap_or_else(|| "not installed".to_owned());
        bail!(
            "Trunk {TRUNK_VERSION} is required; found {found}. Run `cargo run -p bootstrap -- init`"
        );
    }
    Ok(())
}

pub fn wasm_target_installed() -> Result<bool> {
    let installed = output("rustup", &["target", "list", "--installed"])?;
    Ok(installed.lines().any(|line| line.trim() == WASM_TARGET))
}

pub fn trunk_version() -> Result<Option<String>> {
    match Command::new("trunk").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8(output.stdout).context("Trunk version was not UTF-8")?;
            Ok(parse_tool_version(&text, "trunk")
                .ok()
                .map(|version| version.to_string()))
        }
        Ok(output) => bail!("`trunk --version` failed with {}", output.status),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("failed to run trunk --version"),
    }
}

pub fn install_wasm_target() -> Result<()> {
    status("rustup", &["target", "add", WASM_TARGET])
}

pub fn install_trunk(force: bool) -> Result<()> {
    let mut args = vec!["install", "--locked", "trunk@0.21.14"];
    if force {
        args.push("--force");
    }
    status("cargo", &args)
}

fn output(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {program}; is it installed and on PATH?"))?;
    if !output.status.success() {
        bail!("{program} {} failed with {}", args.join(" "), output.status);
    }
    String::from_utf8(output.stdout).with_context(|| format!("{program} output was not UTF-8"))
}

fn status(program: &str, args: &[&str]) -> Result<()> {
    println!("+ {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {program}"))?;
    if !status.success() {
        bail!("{program} {} failed with {status}", args.join(" "));
    }
    Ok(())
}

fn parse_tool_version(output: &str, tool: &str) -> Result<Version> {
    let raw = output
        .split_whitespace()
        .nth(1)
        .with_context(|| format!("could not parse {tool} version from {output:?}"))?;
    Version::parse(raw).with_context(|| format!("could not parse {tool} version {raw:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stable_tool_versions() {
        assert_eq!(
            parse_tool_version("rustc 1.85.0 (hash date)", "rustc").unwrap(),
            MINIMUM_RUST
        );
        assert_eq!(
            parse_tool_version("trunk 0.21.14", "trunk").unwrap(),
            Version::new(0, 21, 14)
        );
    }
}
