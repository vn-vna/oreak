use std::{
    collections::BTreeMap,
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::{FrontendArgs, ServerArgs};

pub const SCHEMA_VERSION: u32 = 1;
pub const CONFIG_FILE: &str = "bootstrap.local.toml";
pub const DEFAULT_SERVER_LISTEN: &str = "127.0.0.1:3000";
pub const DEFAULT_FRONTEND_LISTEN: &str = "127.0.0.1:8080";
pub const DEFAULT_WEB_DIST: &str = "apps/oreak-web/dist";
pub const DEFAULT_RUST_LOG: &str = "info";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
    pub schema_version: u32,
    pub server: ServerConfig,
    pub frontend: FrontendConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub web_dist: PathBuf,
    pub secure_cookies: bool,
    pub rust_log: String,
    #[serde(default)]
    pub release: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontendConfig {
    pub listen: SocketAddr,
    pub open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<SocketAddr>,
    #[serde(default)]
    pub release: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveServer {
    pub release: bool,
    pub listen: SocketAddr,
    pub web_dist: PathBuf,
    pub secure_cookies: bool,
    pub rust_log: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveFrontend {
    pub release: bool,
    pub listen: SocketAddr,
    pub backend: SocketAddr,
    pub open: bool,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            server: ServerConfig {
                listen: DEFAULT_SERVER_LISTEN
                    .parse()
                    .expect("valid default address"),
                web_dist: PathBuf::from(DEFAULT_WEB_DIST),
                secure_cookies: false,
                rust_log: DEFAULT_RUST_LOG.to_owned(),
                release: false,
            },
            frontend: FrontendConfig {
                listen: DEFAULT_FRONTEND_LISTEN
                    .parse()
                    .expect("valid default address"),
                open: true,
                backend: None,
                release: false,
            },
        }
    }
}

pub fn load(root: &Path) -> Result<Option<LocalConfig>> {
    let path = root.join(CONFIG_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    parse(&contents)
        .with_context(|| format!("failed to parse {}", path.display()))
        .map(Some)
}

pub fn parse(contents: &str) -> Result<LocalConfig> {
    let config: LocalConfig = toml::from_str(contents)?;
    if config.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported bootstrap config schema {}; expected {}",
            config.schema_version,
            SCHEMA_VERSION
        );
    }
    if config.server.rust_log.trim().is_empty() {
        bail!("server.rust_log must not be empty");
    }
    Ok(config)
}

pub fn write_if_changed(root: &Path, config: &LocalConfig) -> Result<bool> {
    let path = root.join(CONFIG_FILE);
    let contents = toml::to_string_pretty(config).context("failed to serialize local config")?;
    if fs::read_to_string(&path).ok().as_deref() == Some(contents.as_str()) {
        return Ok(false);
    }
    fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

pub fn environment() -> BTreeMap<String, String> {
    env::vars().collect()
}

pub fn resolve_server(
    args: &ServerArgs,
    local: Option<&LocalConfig>,
    env: &BTreeMap<String, String>,
    root: &Path,
) -> Result<EffectiveServer> {
    let defaults = LocalConfig::default();
    let configured = local.map_or(&defaults.server, |config| &config.server);
    let release = args
        .release
        .unwrap_or(parse_env_bool(env, "OREAK_RELEASE")?.unwrap_or(configured.release));
    let listen = args
        .listen
        .unwrap_or(parse_env(env, "OREAK_LISTEN")?.unwrap_or(configured.listen));
    let web_dist = args
        .web_dist
        .clone()
        .or_else(|| env.get("OREAK_WEB_DIST").map(PathBuf::from))
        .unwrap_or_else(|| configured.web_dist.clone());
    let secure_cookies = args.secure_cookies.unwrap_or(
        parse_env_bool(env, "OREAK_SECURE_COOKIES")?.unwrap_or(configured.secure_cookies),
    );
    let rust_log = args
        .rust_log
        .clone()
        .or_else(|| env.get("RUST_LOG").cloned())
        .unwrap_or_else(|| configured.rust_log.clone());
    if rust_log.trim().is_empty() {
        bail!("RUST_LOG/rust_log must not be empty");
    }
    Ok(EffectiveServer {
        release,
        listen,
        web_dist: absolute(root, &web_dist),
        secure_cookies,
        rust_log,
    })
}

pub fn resolve_frontend(
    args: &FrontendArgs,
    local: Option<&LocalConfig>,
    env: &BTreeMap<String, String>,
) -> Result<EffectiveFrontend> {
    let defaults = LocalConfig::default();
    let configured = local.map_or(&defaults.frontend, |config| &config.frontend);
    let configured_backend = configured
        .backend
        .or_else(|| local.map(|config| config.server.listen))
        .unwrap_or(defaults.server.listen);
    Ok(EffectiveFrontend {
        release: args.release.unwrap_or(
            parse_env_bool(env, "OREAK_FRONTEND_RELEASE")?.unwrap_or(configured.release),
        ),
        listen: args
            .listen
            .unwrap_or(parse_env(env, "OREAK_FRONTEND_LISTEN")?.unwrap_or(configured.listen)),
        backend: args
            .backend
            .unwrap_or(parse_env(env, "OREAK_BACKEND")?.unwrap_or(configured_backend)),
        open: args
            .open
            .unwrap_or(parse_env_bool(env, "OREAK_FRONTEND_OPEN")?.unwrap_or(configured.open)),
    })
}

pub fn validate_bind(address: SocketAddr, acknowledged: bool, label: &str) -> Result<()> {
    if !address.ip().is_loopback() && !acknowledged {
        bail!(
            "{label} address {address} is not loopback; pass --allow-public-bind to acknowledge the exposure"
        );
    }
    Ok(())
}

pub fn addresses_conflict(left: SocketAddr, right: SocketAddr) -> bool {
    left.port() == right.port()
        && (left.ip() == right.ip() || left.ip().is_unspecified() || right.ip().is_unspecified())
}

pub fn connect_address(address: SocketAddr) -> SocketAddr {
    if !address.ip().is_unspecified() {
        return address;
    }
    if address.is_ipv4() {
        SocketAddr::from(([127, 0, 0, 1], address.port()))
    } else {
        SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], address.port()))
    }
}

pub fn public_address(address: SocketAddr) -> SocketAddr {
    connect_address(address)
}

fn absolute(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

fn parse_env<T>(env: &BTreeMap<String, String>, key: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    env.get(key)
        .map(|value| {
            value
                .parse()
                .map_err(|error| anyhow::anyhow!("invalid {key}={value:?}: {error}"))
        })
        .transpose()
}

fn parse_env_bool(env: &BTreeMap<String, String>, key: &str) -> Result<Option<bool>> {
    env.get(key)
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => bail!("invalid {key}={value:?}; expected true or false"),
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_schema_and_rejects_unknown_schema() {
        let encoded = toml::to_string(&LocalConfig::default()).unwrap();
        assert_eq!(parse(&encoded).unwrap().schema_version, SCHEMA_VERSION);
        assert!(parse(&encoded.replace("schema_version = 1", "schema_version = 2")).is_err());
    }

    #[test]
    fn server_precedence_is_cli_then_env_then_config_then_defaults() {
        let mut local = LocalConfig::default();
        local.server.listen = "127.0.0.1:3001".parse().unwrap();
        local.server.rust_log = "warn".into();

        let configured = resolve_server(
            &ServerArgs::default(),
            Some(&local),
            &BTreeMap::new(),
            Path::new("/repo"),
        )
        .unwrap();
        assert_eq!(configured.listen, "127.0.0.1:3001".parse().unwrap());

        let mut env = BTreeMap::new();
        env.insert("OREAK_LISTEN".into(), "127.0.0.1:3002".into());
        env.insert("RUST_LOG".into(), "debug".into());
        let environment = resolve_server(
            &ServerArgs::default(),
            Some(&local),
            &env,
            Path::new("/repo"),
        )
        .unwrap();
        assert_eq!(environment.listen, "127.0.0.1:3002".parse().unwrap());

        let args = ServerArgs {
            listen: Some("127.0.0.1:3003".parse().unwrap()),
            ..ServerArgs::default()
        };

        let resolved = resolve_server(&args, Some(&local), &env, Path::new("/repo")).unwrap();

        assert_eq!(resolved.listen, "127.0.0.1:3003".parse().unwrap());
        assert_eq!(resolved.rust_log, "debug");
        assert!(!resolved.release);

        let defaults = resolve_server(
            &ServerArgs::default(),
            None,
            &BTreeMap::new(),
            Path::new("/repo"),
        )
        .unwrap();
        assert_eq!(defaults.listen, DEFAULT_SERVER_LISTEN.parse().unwrap());
    }

    #[test]
    fn frontend_backend_defaults_to_effective_local_server() {
        let mut local = LocalConfig::default();
        local.server.listen = "127.0.0.1:3900".parse().unwrap();
        let resolved =
            resolve_frontend(&FrontendArgs::default(), Some(&local), &BTreeMap::new()).unwrap();
        assert_eq!(resolved.backend, local.server.listen);
    }

    #[test]
    fn public_and_conflicting_addresses_are_guarded() {
        assert!(validate_bind("0.0.0.0:3000".parse().unwrap(), false, "server").is_err());
        assert!(validate_bind("0.0.0.0:3000".parse().unwrap(), true, "server").is_ok());
        assert!(addresses_conflict(
            "0.0.0.0:3000".parse().unwrap(),
            "127.0.0.1:3000".parse().unwrap()
        ));
    }
}
