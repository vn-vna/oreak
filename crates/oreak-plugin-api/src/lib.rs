//! Serializable contract shared by Oreak plugin hosts and plugin tooling.
//!
//! The contract is deliberately declarative. It contains no host handles for
//! browser, network, filesystem, database, or GPU access.

#![forbid(unsafe_code)]

mod contract;
mod ids;
mod release;
mod validation;

pub use contract::*;
pub use ids::{ContentHash, IdentifierError, LocalId, PluginId, ProjectPluginPin};
pub use release::{
    ReleaseGateEvaluation, ReleaseGateIssue, ReleaseGateIssueKind, evaluate_release_gate,
};
pub use semver::Version;
pub use validation::{Validate, ValidationErrors, ValidationIssue};

/// The only manifest shape understood by this version of the API crate.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// Maximum number of cells a custom entity may occupy.
pub const MAX_OCCUPIED_CELLS: usize = 4_096;
