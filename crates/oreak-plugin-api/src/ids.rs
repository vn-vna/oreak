use std::{fmt, str::FromStr};

use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

use crate::PluginManifest;

const MAX_ID_LEN: usize = 128;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("invalid {kind}: {reason}")]
pub struct IdentifierError {
    kind: &'static str,
    reason: &'static str,
}

impl IdentifierError {
    fn new(kind: &'static str, reason: &'static str) -> Self {
        Self { kind, reason }
    }
}

/// Stable reverse-DNS-style identifier for a plugin.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_identifier("plugin id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for PluginId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for PluginId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PluginId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Identifier scoped to one plugin manifest.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LocalId(String);

impl LocalId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_identifier("local id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for LocalId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for LocalId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LocalId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// A lowercase BLAKE3 digest with an explicit algorithm prefix.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let Some(digest) = value.strip_prefix("blake3:") else {
            return Err(IdentifierError::new(
                "content hash",
                "expected a blake3: prefix",
            ));
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(IdentifierError::new(
                "content hash",
                "expected 64 lowercase hexadecimal digits",
            ));
        }
        Ok(Self(value))
    }

    pub fn blake3(bytes: impl AsRef<[u8]>) -> Self {
        Self(format!("blake3:{}", blake3::hash(bytes.as_ref()).to_hex()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ContentHash {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for ContentHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// An exact project pin. Ranges and floating versions are intentionally absent.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPluginPin {
    pub plugin_id: PluginId,
    pub plugin_version: Version,
    pub api_version: Version,
    pub source_hash: ContentHash,
}

impl ProjectPluginPin {
    pub fn from_manifest(manifest: &PluginManifest) -> Self {
        Self {
            plugin_id: manifest.plugin_id.clone(),
            plugin_version: manifest.plugin_version.clone(),
            api_version: manifest.api_version.clone(),
            source_hash: manifest.source_hash.clone(),
        }
    }

    pub fn matches_manifest(&self, manifest: &PluginManifest) -> bool {
        self == &Self::from_manifest(manifest)
    }
}

fn validate_identifier(kind: &'static str, value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::new(kind, "must not be empty"));
    }
    if value.len() > MAX_ID_LEN {
        return Err(IdentifierError::new(kind, "is longer than 128 bytes"));
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return Err(IdentifierError::new(
            kind,
            "must start with a lowercase ASCII letter",
        ));
    }
    if !bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
    }) {
        return Err(IdentifierError::new(
            kind,
            "contains characters outside [a-z0-9._-]",
        ));
    }
    if matches!(bytes.last(), Some(b'.' | b'-' | b'_'))
        || bytes
            .windows(2)
            .any(|pair| matches!(pair, [b'.' | b'-' | b'_', b'.' | b'-' | b'_']))
    {
        return Err(IdentifierError::new(
            kind,
            "separators must occur between alphanumeric segments",
        ));
    }
    Ok(())
}
