use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tokio::process::Command;

use crate::config::{EffectiveFrontend, EffectiveServer, connect_address, public_address};

pub const TRUNK_VERSION: &str = "0.21.14";
pub const TRUNK_REQUIREMENT: &str = "=0.21.14";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
}

impl CommandSpec {
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(&self.cwd)
            .envs(&self.env)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        command
    }

    pub fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(self.args.iter())
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub fn server(root: &Path, settings: &EffectiveServer) -> CommandSpec {
    let mut args: Vec<OsString> = vec![
        "run".into(),
        "--locked".into(),
        "-p".into(),
        "oreak-server".into(),
    ];
    if settings.release {
        args.push("--release".into());
    }
    CommandSpec {
        program: "cargo".into(),
        args,
        cwd: root.to_owned(),
        env: BTreeMap::from([
            ("OREAK_LISTEN".into(), settings.listen.to_string().into()),
            (
                "OREAK_WEB_DIST".into(),
                settings.web_dist.as_os_str().to_owned(),
            ),
            (
                "OREAK_SECURE_COOKIES".into(),
                settings.secure_cookies.to_string().into(),
            ),
            ("RUST_LOG".into(), settings.rust_log.clone().into()),
        ]),
    }
}

pub fn frontend(
    root: &Path,
    config_path: &Path,
    settings: &EffectiveFrontend,
    watch: bool,
) -> CommandSpec {
    let mut args: Vec<OsString> = vec![
        if watch { "watch" } else { "serve" }.into(),
        "--config".into(),
        config_path.as_os_str().to_owned(),
        "--locked".into(),
    ];
    if settings.release {
        args.push("--release".into());
    }
    CommandSpec {
        program: "trunk".into(),
        args,
        cwd: root.join("apps/oreak-web"),
        env: BTreeMap::new(),
    }
}

pub fn server_build(root: &Path, release: bool) -> CommandSpec {
    let mut args: Vec<OsString> = vec![
        "build".into(),
        "--locked".into(),
        "-p".into(),
        "oreak-server".into(),
    ];
    if release {
        args.push("--release".into());
    }
    simple(root, "cargo", args)
}

pub fn trunk_build(root: &Path, release: bool) -> CommandSpec {
    let mut args: Vec<OsString> = vec![
        "build".into(),
        "--config".into(),
        root.join("apps/oreak-web/Trunk.toml").into_os_string(),
        "--locked".into(),
    ];
    if release {
        args.push("--release".into());
    }
    simple(&root.join("apps/oreak-web"), "trunk", args)
}

pub fn verification(root: &Path) -> Vec<CommandSpec> {
    vec![
        simple(root, "cargo", strings(&["fmt", "--all", "--check"])),
        simple(
            root,
            "cargo",
            strings(&["check", "--locked", "--workspace", "--all-targets"]),
        ),
        simple(
            root,
            "cargo",
            strings(&[
                "clippy",
                "--locked",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]),
        ),
        simple(root, "cargo", strings(&["test", "--locked", "--workspace"])),
        simple(
            root,
            "cargo",
            strings(&[
                "check",
                "--locked",
                "-p",
                "oreak-core",
                "-p",
                "oreak-project",
                "-p",
                "oreak-plugin-api",
                "-p",
                "oreak-protocol",
                "-p",
                "oreak-web",
                "--target",
                "wasm32-unknown-unknown",
            ]),
        ),
        trunk_build(root, true),
    ]
}

pub fn generated_trunk(root: &Path, settings: &EffectiveFrontend) -> Result<String> {
    let frontend_authority = public_address(settings.listen).to_string();
    let backend_authority = connect_address(settings.backend).to_string();
    let host = BTreeMap::from([("Host".to_owned(), frontend_authority)]);
    let config = TrunkConfig {
        trunk_version: TRUNK_REQUIREMENT,
        build: TrunkBuild {
            target: root.join("apps/oreak-web/index.html"),
            dist: root.join("target/bootstrap/frontend-dist"),
            public_url: "/",
            filehash: true,
            release: settings.release,
        },
        serve: TrunkServe {
            addresses: vec![settings.listen.ip().to_string()],
            port: settings.listen.port(),
            open: settings.open,
        },
        proxy: vec![
            TrunkProxy {
                backend: format!("http://{backend_authority}/api/"),
                request_headers: host.clone(),
                ws: false,
                no_system_proxy: true,
            },
            TrunkProxy {
                backend: format!("ws://{backend_authority}/rpc"),
                request_headers: host,
                ws: true,
                no_system_proxy: true,
            },
        ],
    };
    toml::to_string_pretty(&config).context("failed to serialize generated Trunk config")
}

pub fn write_generated_trunk(root: &Path, settings: &EffectiveFrontend) -> Result<PathBuf> {
    let directory = root.join("target/bootstrap");
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let path = directory.join("Trunk.dev.toml");
    let contents = generated_trunk(root, settings)?;
    std::fs::write(&path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

pub async fn run_checked(spec: &CommandSpec) -> Result<()> {
    println!("+ {}", spec.display());
    let status = spec
        .command()
        .status()
        .await
        .with_context(|| format!("failed to start {}", spec.program.to_string_lossy()))?;
    if !status.success() {
        bail!("command failed with {status}: {}", spec.display());
    }
    Ok(())
}

fn simple(root: &Path, program: &str, args: Vec<OsString>) -> CommandSpec {
    CommandSpec {
        program: program.into(),
        args,
        cwd: root.to_owned(),
        env: BTreeMap::new(),
    }
}

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[derive(Serialize)]
struct TrunkConfig<'a> {
    #[serde(rename = "trunk-version")]
    trunk_version: &'a str,
    build: TrunkBuild<'a>,
    serve: TrunkServe,
    proxy: Vec<TrunkProxy>,
}

#[derive(Serialize)]
struct TrunkBuild<'a> {
    target: PathBuf,
    dist: PathBuf,
    public_url: &'a str,
    filehash: bool,
    release: bool,
}

#[derive(Serialize)]
struct TrunkServe {
    addresses: Vec<String>,
    port: u16,
    open: bool,
}

#[derive(Serialize)]
struct TrunkProxy {
    backend: String,
    request_headers: BTreeMap<String, String>,
    ws: bool,
    no_system_proxy: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_command_runs_from_root_with_effective_environment() {
        let root = Path::new("repo");
        let settings = EffectiveServer {
            release: true,
            listen: "127.0.0.1:3100".parse().unwrap(),
            web_dist: root.join("dist"),
            secure_cookies: true,
            rust_log: "debug".into(),
        };
        let command = server(root, &settings);
        assert_eq!(command.cwd, root);
        assert_eq!(
            command.args,
            strings(&["run", "--locked", "-p", "oreak-server", "--release"])
        );
        assert_eq!(
            command.env.get(&OsString::from("OREAK_LISTEN")).unwrap(),
            "127.0.0.1:3100"
        );
    }

    #[test]
    fn generated_config_pins_trunk_and_overrides_host_for_both_proxies() {
        let settings = EffectiveFrontend {
            release: false,
            listen: "127.0.0.1:8080".parse().unwrap(),
            backend: "127.0.0.1:3000".parse().unwrap(),
            open: false,
        };
        let encoded = generated_trunk(Path::new("/repo"), &settings).unwrap();
        let parsed: toml::Value = toml::from_str(&encoded).unwrap();
        assert_eq!(parsed["trunk-version"].as_str(), Some(TRUNK_REQUIREMENT));
        let proxies = parsed["proxy"].as_array().unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(
            proxies[0]["backend"].as_str(),
            Some("http://127.0.0.1:3000/api/")
        );
        assert_eq!(
            proxies[1]["backend"].as_str(),
            Some("ws://127.0.0.1:3000/rpc")
        );
        assert_eq!(
            proxies[0]["request_headers"]["Host"].as_str(),
            Some("127.0.0.1:8080")
        );
        assert_eq!(
            proxies[1]["request_headers"]["Host"].as_str(),
            Some("127.0.0.1:8080")
        );
        assert_eq!(proxies[1]["ws"].as_bool(), Some(true));
        assert_eq!(proxies[0]["no_system_proxy"].as_bool(), Some(true));
    }

    #[test]
    fn frontend_command_selects_serve_or_watch() {
        let root = Path::new("repo");
        let config = root.join("target/bootstrap/Trunk.dev.toml");
        let settings = EffectiveFrontend {
            release: true,
            listen: "127.0.0.1:8080".parse().unwrap(),
            backend: "127.0.0.1:3000".parse().unwrap(),
            open: false,
        };

        let serve = frontend(root, &config, &settings, false);
        let watch = frontend(root, &config, &settings, true);

        assert_eq!(serve.args[0], "serve");
        assert_eq!(watch.args[0], "watch");
        assert_eq!(serve.cwd, root.join("apps/oreak-web"));
        assert!(serve.args.contains(&OsString::from("--release")));
    }

    #[test]
    fn verification_matches_ci_order() {
        let commands = verification(Path::new("repo"));
        assert_eq!(commands.len(), 6);
        assert_eq!(commands[0].args, strings(&["fmt", "--all", "--check"]));
        assert_eq!(commands[5].program, OsString::from("trunk"));
        assert!(commands[5].args.contains(&OsString::from("--release")));
    }
}
