//! Shared wire types and generated JSON-RPC interfaces for Oreak collaboration.

#![forbid(unsafe_code)]

use std::fmt;

use jsonrpsee::{
    core::{RpcResult, SubscriptionResult},
    proc_macros::rpc,
};
use oreak_core::{ActorId, CommandId, GridPoint, ImagePlacement, PlaceableEntity};
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

/// Applies a server-prepared immutable image to the exact Pools used for preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyImageRequest {
    pub target: ProjectLevelTarget,
    pub metadata: CommandMetadata,
    pub prepared_image_id: String,
    pub placement: ImagePlacement,
    pub targets: Vec<PlaceableEntity>,
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PresenceId(String);

impl PresenceId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceParticipant {
    pub id: PresenceId,
    pub actor: ActorId,
    pub cursor: Option<GridPoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LevelPresenceItem {
    Snapshot {
        target: ProjectLevelTarget,
        self_id: PresenceId,
        participants: Vec<PresenceParticipant>,
    },
    Joined {
        target: ProjectLevelTarget,
        participant: PresenceParticipant,
    },
    Left {
        target: ProjectLevelTarget,
        presence_id: PresenceId,
    },
    Cursor {
        target: ProjectLevelTarget,
        presence_id: PresenceId,
        cursor: Option<GridPoint>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateLevelCursorRequest {
    pub target: ProjectLevelTarget,
    pub cursor: Option<GridPoint>,
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

    #[method(name = "apply_image", param_kind = map, with_extensions)]
    async fn apply_image(&self, request: ApplyImageRequest) -> RpcResult<ApplyCommandResponse>;

    #[method(name = "undo_latest", param_kind = map, with_extensions)]
    async fn undo_latest(&self, request: UndoLatestRequest) -> RpcResult<UndoLatestResponse>;

    #[method(name = "update_level_cursor", param_kind = map, with_extensions)]
    async fn update_level_cursor(&self, request: UpdateLevelCursorRequest) -> RpcResult<()>;

    #[subscription(
        name = "subscribe_level",
        unsubscribe = "unsubscribe_level",
        item = LevelSubscriptionItem,
        param_kind = map,
        with_extensions
    )]
    async fn subscribe_level(&self, target: ProjectLevelTarget) -> SubscriptionResult;

    #[subscription(
        name = "subscribe_level_presence",
        unsubscribe = "unsubscribe_level_presence",
        item = LevelPresenceItem,
        param_kind = map,
        with_extensions
    )]
    async fn subscribe_level_presence(&self, target: ProjectLevelTarget) -> SubscriptionResult;
}

#[cfg(test)]
mod tests {
    use super::{
        ApplyCommandRequest, LevelHistoryRequest, LevelId, LevelPresenceItem, PresenceId,
        PresenceParticipant, ProjectId, ProjectLevelTarget, RpcErrorCode, RpcErrorData,
    };
    use oreak_core::{
        ActorId, BlindPixel, BlindStroke, CellEdit, CellKind, CommandEnvelope, CommandMetadata,
        EntityId, EntityMove, GridAnchor, GridPoint, GridSize, LevelCommand,
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
    fn presence_wire_shapes_are_scoped_and_string_identified() {
        let target = ProjectLevelTarget::new("project-one", "level-one");
        let item = LevelPresenceItem::Snapshot {
            target: target.clone(),
            self_id: PresenceId::new("connection-one"),
            participants: vec![PresenceParticipant {
                id: PresenceId::new("connection-one"),
                actor: ActorId::new("user-one"),
                cursor: Some(GridPoint::new(2, 3)),
            }],
        };
        assert_eq!(
            serde_json::to_value(item).unwrap(),
            serde_json::json!({
                "kind": "snapshot",
                "target": { "project_id": "project-one", "level_id": "level-one" },
                "self_id": "connection-one",
                "participants": [{
                    "id": "connection-one",
                    "actor": "user-one",
                    "cursor": { "x": 2, "y": 3 }
                }]
            })
        );
        assert_eq!(
            serde_json::to_value(LevelPresenceItem::Cursor {
                target,
                presence_id: PresenceId::new("connection-one"),
                cursor: None,
            })
            .unwrap()["cursor"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn blind_brush_commands_have_stable_wire_shapes() {
        let target = ProjectLevelTarget::new("project-one", "level-one");
        let metadata = CommandMetadata::new("brush-1", "client-actor", 1_000);
        let request = ApplyCommandRequest {
            target,
            command: CommandEnvelope::new(
                metadata,
                LevelCommand::PaintBlindStroke {
                    entity_id: EntityId::from("blind-1"),
                    color_index: 4,
                    stroke: BlindStroke::new(vec![
                        BlindPixel::new(1, 0),
                        BlindPixel::new(0, 0),
                        BlindPixel::new(1, 0),
                    ])
                    .unwrap(),
                },
            ),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            value["command"]["command"]["PaintBlindStroke"],
            serde_json::json!({
                "entity_id": "blind-1",
                "color_index": 4,
                "stroke": { "pixels": [{ "x": 0, "y": 0 }, { "x": 1, "y": 0 }] }
            })
        );
        assert_eq!(
            serde_json::from_value::<ApplyCommandRequest>(value).unwrap(),
            request
        );

        let erase = LevelCommand::EraseBlindStroke {
            entity_id: EntityId::from("blind-1"),
            stroke: BlindStroke::new(vec![BlindPixel::new(2, 3)]).unwrap(),
        };
        assert_eq!(
            serde_json::to_value(erase).unwrap(),
            serde_json::json!({
                "EraseBlindStroke": {
                    "entity_id": "blind-1",
                    "stroke": { "pixels": [{ "x": 2, "y": 3 }] }
                }
            })
        );
        let fill = LevelCommand::FloodFillBlind {
            entity_id: EntityId::from("blind-1"),
            start: BlindPixel::new(3, 7),
            color_index: 6,
        };
        assert_eq!(
            serde_json::to_value(fill).unwrap(),
            serde_json::json!({
                "FloodFillBlind": {
                    "entity_id": "blind-1",
                    "start": { "x": 3, "y": 7 },
                    "color_index": 6
                }
            })
        );
    }

    #[test]
    fn resize_and_group_move_commands_have_stable_ordered_wire_shapes() {
        let resize = LevelCommand::ResizeGrid {
            size: GridSize::new(12, 7).unwrap(),
            anchor: GridAnchor::TopLeft,
        };
        assert_eq!(
            serde_json::to_value(&resize).unwrap(),
            serde_json::json!({
                "ResizeGrid": {
                    "size": { "width": 12, "height": 7 },
                    "anchor": "TopLeft"
                }
            })
        );

        let moves = LevelCommand::MoveEntities {
            moves: vec![
                EntityMove::new("second", GridPoint::new(4, 3)),
                EntityMove::new("first", GridPoint::new(2, 1)),
            ],
        };
        let value = serde_json::to_value(&moves).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "MoveEntities": {
                    "moves": [
                        { "entity_id": "second", "origin": { "x": 4, "y": 3 } },
                        { "entity_id": "first", "origin": { "x": 2, "y": 1 } }
                    ]
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<LevelCommand>(value).unwrap(),
            moves
        );
    }

    #[test]
    fn multi_cell_stamp_has_a_stable_wire_shape() {
        let command = LevelCommand::SetCells {
            cells: vec![
                CellEdit::new(GridPoint::new(1, 2), CellKind::Wall),
                CellEdit::new(GridPoint::new(2, 2), CellKind::Floor),
            ],
        };
        let value = serde_json::to_value(&command).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "SetCells": {
                    "cells": [
                        { "point": { "x": 1, "y": 2 }, "kind": "Wall" },
                        { "point": { "x": 2, "y": 2 }, "kind": "Floor" }
                    ]
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<LevelCommand>(value).unwrap(),
            command
        );
    }

    #[test]
    fn apply_image_request_roundtrips_expected_targets_without_embedded_asset() {
        let target = oreak_core::PlaceableEntity::blind(
            "pool",
            GridPoint::new(0, 0),
            oreak_core::Shape::new(1, 1, 1).unwrap(),
            oreak_core::Blind::new(1, vec![oreak_core::BlindTile::empty(1).unwrap()]).unwrap(),
        )
        .unwrap();
        let request = super::ApplyImageRequest {
            target: ProjectLevelTarget::new("project", "level"),
            metadata: oreak_core::CommandMetadata::new("image", "artist", 100),
            prepared_image_id: "prepared-1".to_owned(),
            placement: oreak_core::ImagePlacement {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                sampling: oreak_core::ImageSampling::Nearest,
                pixelation: 1,
                resolution: None,
                transparency: oreak_core::ImageTransparency::Preserve,
            },
            targets: vec![target],
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["prepared_image_id"], "prepared-1");
        assert_eq!(value["placement"]["sampling"], "nearest");
        assert_eq!(value["targets"].as_array().unwrap().len(), 1);
        assert!(value.get("image").is_none());
        assert!(value.get("settings").is_none());
        assert_eq!(
            serde_json::from_value::<super::ApplyImageRequest>(value).unwrap(),
            request
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
