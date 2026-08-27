//! Shared wire types and generated JSON-RPC interfaces for Oreak collaboration.

#![forbid(unsafe_code)]

use std::fmt;

use jsonrpsee::{
    core::{RpcResult, SubscriptionResult},
    proc_macros::rpc,
};
use oreak_core::{ActorId, CommandId};
pub use oreak_core::{
    ApplyOutcome, CommandEnvelope, CommandMetadata, HistoryEvent, LevelHash, LevelSnapshot,
};
use serde::{Deserialize, Serialize};

/// Maximum number of historical events returned by one `level_history` page.
pub const MAX_LEVEL_HISTORY_PAGE_SIZE: u16 = 128;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LevelId(String);

impl LevelId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LevelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for LevelId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for LevelId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl ProjectId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for ProjectId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ProjectId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProjectLevelTarget {
    pub project_id: ProjectId,
    pub level_id: LevelId,
}

impl ProjectLevelTarget {
    #[must_use]
    pub fn new(project_id: impl Into<ProjectId>, level_id: impl Into<LevelId>) -> Self {
        Self {
            project_id: project_id.into(),
            level_id: level_id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelSnapshotResponse {
    pub target: ProjectLevelTarget,
    pub server_sequence: u64,
    pub snapshot: LevelSnapshot,
    /// Lowercase hexadecimal representation of the core `LevelHash`.
    pub level_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelHistoryRequest {
    pub target: ProjectLevelTarget,
    /// Return only events whose sequence is strictly less than this cursor.
    /// `None` starts at the newest event currently stored by the server.
    pub before_sequence: Option<u64>,
    /// Requested page size. The server rejects zero and caps larger values at
    /// [`MAX_LEVEL_HISTORY_PAGE_SIZE`].
    pub limit: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelHistoryResponse {
    pub target: ProjectLevelTarget,
    /// Events are ordered by strictly descending server sequence (newest first).
    pub events: Vec<HistoryEvent>,
    /// Exclusive cursor for the next older page. Present exactly when
    /// `has_more` is true.
    pub next_before_sequence: Option<u64>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyCommandRequest {
    pub target: ProjectLevelTarget,
    pub command: CommandEnvelope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApplyCommandResult {
    Applied { event: Box<HistoryEvent> },
    NoChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyCommandResponse {
    pub target: ProjectLevelTarget,
    pub server_sequence: u64,
    pub level_hash: String,
    pub result: ApplyCommandResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoLatestRequest {
    pub target: ProjectLevelTarget,
    pub metadata: CommandMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoLatestResponse {
    pub target: ProjectLevelTarget,
    pub server_sequence: u64,
    pub level_hash: String,
    pub event: HistoryEvent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelEvent {
    pub target: ProjectLevelTarget,
    pub server_sequence: u64,
    pub level_hash: String,
    pub event: HistoryEvent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LevelSubscriptionItem {
    Snapshot {
        snapshot: Box<LevelSnapshotResponse>,
    },
    Event {
        event: Box<LevelEvent>,
    },
    ResyncRequired {
        target: ProjectLevelTarget,
        missed_events: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcErrorCode {
    DuplicateCommand,
    UndoConflict,
    NothingToUndo,
    InvalidCommand,
    AuthenticationRequired,
    PermissionDenied,
    LevelUnavailable,
    Internal,
}

impl RpcErrorCode {
    #[must_use]
    pub const fn json_rpc_code(self) -> i32 {
        match self {
            Self::DuplicateCommand => -32010,
            Self::UndoConflict => -32011,
            Self::NothingToUndo => -32012,
            Self::InvalidCommand => -32013,
            Self::AuthenticationRequired => -32020,
            Self::PermissionDenied => -32021,
            Self::LevelUnavailable => -32022,
            Self::Internal => -32603,
        }
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::DuplicateCommand => "command ID was already accepted",
            Self::UndoConflict => "undo is blocked by later active changes",
            Self::NothingToUndo => "actor has no active command to undo",
            Self::InvalidCommand => "command is invalid for the current level",
            Self::AuthenticationRequired => "authentication required",
            Self::PermissionDenied => "permission denied",
            Self::LevelUnavailable => "project level is unavailable",
            Self::Internal => "internal server error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RpcErrorDetails {
    DuplicateCommand {
        command_id: CommandId,
    },
    UndoConflict {
        actor: ActorId,
        blocked_sequences: Vec<u64>,
    },
    NothingToUndo {
        actor: ActorId,
    },
    InvalidCommand {
        reason: String,
    },
    AuthenticationRequired,
    PermissionDenied,
    LevelUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcErrorData {
    pub code: RpcErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<RpcErrorDetails>,
}

impl RpcErrorData {
    #[must_use]
    pub fn new(code: RpcErrorCode, details: Option<RpcErrorDetails>) -> Self {
        Self {
            code,
            message: code.message().to_owned(),
            details,
        }
    }
}

#[rpc(client, server)]
pub trait OreakRpc {
    #[method(name = "health")]
    async fn health(&self) -> RpcResult<HealthResponse>;

    #[method(name = "level_snapshot", param_kind = map, with_extensions)]
    async fn level_snapshot(&self, target: ProjectLevelTarget) -> RpcResult<LevelSnapshotResponse>;

    #[method(name = "level_history", param_kind = map, with_extensions)]
    async fn level_history(&self, request: LevelHistoryRequest) -> RpcResult<LevelHistoryResponse>;

    #[method(name = "apply_command", param_kind = map, with_extensions)]
    async fn apply_command(&self, request: ApplyCommandRequest) -> RpcResult<ApplyCommandResponse>;

    #[method(name = "undo_latest", param_kind = map, with_extensions)]
    async fn undo_latest(&self, request: UndoLatestRequest) -> RpcResult<UndoLatestResponse>;

    #[subscription(
        name = "subscribe_level",
        unsubscribe = "unsubscribe_level",
        item = LevelSubscriptionItem,
        param_kind = map,
        with_extensions
    )]
    async fn subscribe_level(&self, target: ProjectLevelTarget) -> SubscriptionResult;
}

#[cfg(test)]
mod tests {
    use super::{
        LevelHistoryRequest, LevelId, ProjectId, ProjectLevelTarget, RpcErrorCode, RpcErrorData,
    };

    #[test]
    fn level_id_is_a_transparent_string() {
        let id = LevelId::from("level-one");
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""level-one""#);
        assert_eq!(id.as_str(), "level-one");
    }

    #[test]
    fn project_level_target_has_explicit_scope() {
        let target = ProjectLevelTarget::new("project-one", "level-one");
        assert_eq!(target.project_id, ProjectId::from("project-one"));
        assert_eq!(target.level_id, LevelId::from("level-one"));
        assert_eq!(
            serde_json::to_string(&target).unwrap(),
            r#"{"project_id":"project-one","level_id":"level-one"}"#
        );
    }

    #[test]
    fn error_code_and_message_are_stable() {
        let error = RpcErrorData::new(RpcErrorCode::DuplicateCommand, None);
        assert_eq!(error.code.json_rpc_code(), -32010);
        assert_eq!(error.message, "command ID was already accepted");
    }

    #[test]
    fn history_request_has_an_explicit_exclusive_cursor() {
        let request = LevelHistoryRequest {
            target: ProjectLevelTarget::new("project-one", "level-one"),
            before_sequence: Some(42),
            limit: 25,
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"target":{"project_id":"project-one","level_id":"level-one"},"before_sequence":42,"limit":25}"#
        );
    }
}
