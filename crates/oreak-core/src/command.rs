use serde::{Deserialize, Serialize};

use crate::{
    ActorId, BlindGuide, BlindGuidePatch, BlindGuideSet, BlindPixel, BlindStroke, BlindTile,
    BlindTilePatch, CellKind, CommandId, Decorator, DecoratorId, DirectionMode, EntityId,
    GridPoint, LevelHash, PlaceableEntity, ShapeCell,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandMetadata {
    pub id: CommandId,
    pub actor: ActorId,
    pub occurred_at_ms: i64,
}

impl CommandMetadata {
    #[must_use]
    pub fn new(id: impl Into<CommandId>, actor: impl Into<ActorId>, occurred_at_ms: i64) -> Self {
        Self {
            id: id.into(),
            actor: actor.into(),
            occurred_at_ms,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LevelCommandKind {
    SetCell,
    PlaceEntity,
    MoveEntity,
    RotateEntityClockwise,
    FlipEntityHorizontal,
    DeleteEntity,
    RestoreEntity,
    RestoreDeletedEntity,
    ToggleIce,
    SetIce,
    CycleDirection,
    SetDirection,
    AssignKeyLocker,
    RemoveKeyLocker,
    RestoreDecorator,
    PaintBlindStroke,
    EraseBlindStroke,
    FloodFillBlind,
    AddBlindGuides,
    RemoveBlindGuides,
    RestoreBlindPatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LevelCommand {
    SetCell {
        point: GridPoint,
        kind: CellKind,
    },
    PlaceEntity {
        entity: PlaceableEntity,
    },
    MoveEntity {
        entity_id: EntityId,
        origin: GridPoint,
    },
    RotateEntityClockwise {
        entity_id: EntityId,
    },
    FlipEntityHorizontal {
        entity_id: EntityId,
    },
    DeleteEntity {
        entity_id: EntityId,
    },
    /// Exact entity restoration used by compensating history events.
    #[doc(hidden)]
    RestoreEntity {
        entity: PlaceableEntity,
    },
    #[doc(hidden)]
    RestoreDeletedEntity {
        entity: PlaceableEntity,
        decorators: Vec<Decorator>,
    },
    ToggleIce {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },
    SetIce {
        decorator_id: DecoratorId,
        entity_id: EntityId,
        blocking_count: u32,
    },
    CycleDirection {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },
    SetDirection {
        decorator_id: DecoratorId,
        entity_id: EntityId,
        mode: DirectionMode,
    },
    AssignKeyLocker {
        decorator_id: DecoratorId,
        key_entity_id: EntityId,
        lock_entity_id: EntityId,
    },
    RemoveKeyLocker {
        decorator_id: DecoratorId,
    },
    #[doc(hidden)]
    RestoreDecorator {
        decorator_id: DecoratorId,
        decorator: Option<Decorator>,
    },
    PaintBlindStroke {
        entity_id: EntityId,
        color_index: u8,
        stroke: BlindStroke,
    },
    EraseBlindStroke {
        entity_id: EntityId,
        stroke: BlindStroke,
    },
    FloodFillBlind {
        entity_id: EntityId,
        start: BlindPixel,
        color_index: u8,
    },
    AddBlindGuides {
        entity_id: EntityId,
        guides: BlindGuideSet,
    },
    RemoveBlindGuides {
        entity_id: EntityId,
        guides: BlindGuideSet,
    },
    #[doc(hidden)]
    RestoreBlindPatch {
        entity_id: EntityId,
        tile_patches: Vec<BlindTilePatch>,
        guide_patches: Vec<BlindGuidePatch>,
    },
}

impl LevelCommand {
    #[must_use]
    pub const fn kind(&self) -> LevelCommandKind {
        match self {
            Self::SetCell { .. } => LevelCommandKind::SetCell,
            Self::PlaceEntity { .. } => LevelCommandKind::PlaceEntity,
            Self::MoveEntity { .. } => LevelCommandKind::MoveEntity,
            Self::RotateEntityClockwise { .. } => LevelCommandKind::RotateEntityClockwise,
            Self::FlipEntityHorizontal { .. } => LevelCommandKind::FlipEntityHorizontal,
            Self::DeleteEntity { .. } => LevelCommandKind::DeleteEntity,
            Self::RestoreEntity { .. } => LevelCommandKind::RestoreEntity,
            Self::RestoreDeletedEntity { .. } => LevelCommandKind::RestoreDeletedEntity,
            Self::ToggleIce { .. } => LevelCommandKind::ToggleIce,
            Self::SetIce { .. } => LevelCommandKind::SetIce,
            Self::CycleDirection { .. } => LevelCommandKind::CycleDirection,
            Self::SetDirection { .. } => LevelCommandKind::SetDirection,
            Self::AssignKeyLocker { .. } => LevelCommandKind::AssignKeyLocker,
            Self::RemoveKeyLocker { .. } => LevelCommandKind::RemoveKeyLocker,
            Self::RestoreDecorator { .. } => LevelCommandKind::RestoreDecorator,
            Self::PaintBlindStroke { .. } => LevelCommandKind::PaintBlindStroke,
            Self::EraseBlindStroke { .. } => LevelCommandKind::EraseBlindStroke,
            Self::FloodFillBlind { .. } => LevelCommandKind::FloodFillBlind,
            Self::AddBlindGuides { .. } => LevelCommandKind::AddBlindGuides,
            Self::RemoveBlindGuides { .. } => LevelCommandKind::RemoveBlindGuides,
            Self::RestoreBlindPatch { .. } => LevelCommandKind::RestoreBlindPatch,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub metadata: CommandMetadata,
    pub command: LevelCommand,
}

impl CommandEnvelope {
    #[must_use]
    pub const fn new(metadata: CommandMetadata, command: LevelCommand) -> Self {
        Self { metadata, command }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LevelTarget {
    Cell(GridPoint),
    Entity(EntityId),
    Decorator(DecoratorId),
    BlindTile {
        entity_id: EntityId,
        cell: ShapeCell,
    },
    BlindGuide {
        entity_id: EntityId,
        guide: BlindGuide,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LevelValue {
    Cell(CellKind),
    Entity(Option<PlaceableEntity>),
    Decorator(Option<Decorator>),
    BlindTile(Option<BlindTile>),
    BlindGuide(bool),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryChange {
    pub target: LevelTarget,
    pub before: LevelValue,
    pub after: LevelValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub sequence: u64,
    pub metadata: CommandMetadata,
    pub command: LevelCommand,
    pub inverse: LevelCommand,
    pub changes: Vec<HistoryChange>,
    pub reverts_sequence: Option<u64>,
    pub before_hash: LevelHash,
    pub after_hash: LevelHash,
}
