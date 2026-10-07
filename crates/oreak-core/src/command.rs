use serde::{Deserialize, Serialize};

use crate::{
    ActorId, BlindGuide, BlindGuidePatch, BlindGuideSet, BlindPixel, BlindStroke, BlindTile,
    BlindTilePatch, CellKind, CollectCapacity, CollectLayer, CommandId, Decorator, DecoratorId,
    DirectionMode, DistributionRequest, EntityId, GridPoint, GridSize, ImagePlacement,
    IndexedImage, LevelHash, LevelSnapshot, PaletteSettings, PlaceableEntity,
    PoolDistributionGroup, ShapeCell,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridAnchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellEdit {
    pub point: GridPoint,
    pub kind: CellKind,
}

impl CellEdit {
    #[must_use]
    pub const fn new(point: GridPoint, kind: CellKind) -> Self {
        Self { point, kind }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityMove {
    pub entity_id: EntityId,
    pub origin: GridPoint,
}

impl EntityMove {
    #[must_use]
    pub fn new(entity_id: impl Into<EntityId>, origin: GridPoint) -> Self {
        Self {
            entity_id: entity_id.into(),
            origin,
        }
    }
}

/// A semantic entity transformation applied to the entity's current server state.
///
/// Keeping the operation separate from an entity snapshot means concurrent paint
/// and capacity edits are retained when a collaborator moves or rotates an entity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityTransform {
    pub entity_id: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<GridPoint>,
    #[serde(default)]
    pub clockwise_turns: u8,
    #[serde(default)]
    pub flip_horizontal: bool,
}

impl EntityTransform {
    #[must_use]
    pub fn new(entity_id: impl Into<EntityId>) -> Self {
        Self {
            entity_id: entity_id.into(),
            origin: None,
            clockwise_turns: 0,
            flip_horizontal: false,
        }
    }

    #[must_use]
    pub fn moved_to(mut self, origin: GridPoint) -> Self {
        self.origin = Some(origin);
        self
    }

    #[must_use]
    pub fn rotated_clockwise(mut self, turns: u8) -> Self {
        self.clockwise_turns = turns;
        self
    }

    #[must_use]
    pub fn flipped_horizontal(mut self) -> Self {
        self.flip_horizontal = true;
        self
    }
}

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
    ResizeGrid,
    RestoreSnapshot,
    SetCell,
    SetCells,
    PlaceEntity,
    PlaceEntities,
    MoveEntity,
    MoveEntities,
    TransformEntities,
    RotateEntityClockwise,
    FlipEntityHorizontal,
    SetBlindResolution,
    SetBlindResolutions,
    ApplyImageToPools,
    SetPoolDistributionGroups,
    ApplyDistribution,
    SetBlockCollectLayers,
    SetBlockLayerCapacity,
    DeleteEntity,
    DeleteEntities,
    RestoreEntity,
    RestoreEntities,
    RestoreDeletedEntity,
    RestoreDeletedEntities,
    ToggleIce,
    SetIce,
    ToggleGlass,
    SetGlass,
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
    ResizeGrid {
        size: GridSize,
        anchor: GridAnchor,
    },
    /// Exact snapshot restoration used by compensating history events.
    #[doc(hidden)]
    RestoreSnapshot {
        snapshot: Box<LevelSnapshot>,
    },
    SetCell {
        point: GridPoint,
        kind: CellKind,
    },
    SetCells {
        cells: Vec<CellEdit>,
    },
    PlaceEntity {
        entity: PlaceableEntity,
    },
    PlaceEntities {
        entities: Vec<PlaceableEntity>,
    },
    MoveEntity {
        entity_id: EntityId,
        origin: GridPoint,
    },
    MoveEntities {
        moves: Vec<EntityMove>,
    },
    /// Atomically applies movement/rotation/flip operations to the current entities.
    ///
    /// `entities` remains only to replay history emitted by older clients. New writers
    /// must send `transforms`, so concurrent non-geometric edits are not overwritten.
    TransformEntities {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        transforms: Vec<EntityTransform>,
        #[doc(hidden)]
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entities: Vec<PlaceableEntity>,
    },
    RotateEntityClockwise {
        entity_id: EntityId,
    },
    FlipEntityHorizontal {
        entity_id: EntityId,
    },
    SetBlindResolution {
        entity_id: EntityId,
        pixels_per_cell: u8,
    },
    SetBlindResolutions {
        entity_ids: Vec<EntityId>,
        pixels_per_cell: u8,
    },
    /// Applies a prepared image atomically, only while every expected Pool is unchanged.
    /// Image assets are resolved by the server's dedicated `apply_image` endpoint.
    ApplyImageToPools {
        image: IndexedImage,
        settings: PaletteSettings,
        placement: ImagePlacement,
        targets: Vec<PlaceableEntity>,
    },
    /// Saves authoring-only Pool groups if the captured Pool is still unchanged.
    SetPoolDistributionGroups {
        entity_id: EntityId,
        expected: PlaceableEntity,
        groups: Vec<PoolDistributionGroup>,
    },
    /// Recomputes a deterministic weighted allocation from authoritative source pixels.
    /// Every selected source and destination is guarded by an exact captured snapshot.
    ApplyDistribution {
        request: DistributionRequest,
        expected_entities: Vec<PlaceableEntity>,
    },
    SetBlockCollectLayers {
        entity_id: EntityId,
        layers: Vec<CollectLayer>,
    },
    SetBlockLayerCapacity {
        entity_ids: Vec<EntityId>,
        layer_index: usize,
        capacity: CollectCapacity,
    },
    DeleteEntity {
        entity_id: EntityId,
    },
    DeleteEntities {
        entity_ids: Vec<EntityId>,
    },
    /// Exact entity restoration used by compensating history events.
    #[doc(hidden)]
    RestoreEntity {
        entity: PlaceableEntity,
    },
    /// Exact multi-entity restoration used by compensating history events.
    #[doc(hidden)]
    RestoreEntities {
        entities: Vec<PlaceableEntity>,
    },
    #[doc(hidden)]
    RestoreDeletedEntity {
        entity: PlaceableEntity,
        decorators: Vec<Decorator>,
    },
    /// Exact multi-entity/decorator restoration used by compensating history events.
    #[doc(hidden)]
    RestoreDeletedEntities {
        entities: Vec<PlaceableEntity>,
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
    ToggleGlass {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },
    SetGlass {
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
            Self::ResizeGrid { .. } => LevelCommandKind::ResizeGrid,
            Self::RestoreSnapshot { .. } => LevelCommandKind::RestoreSnapshot,
            Self::SetCell { .. } => LevelCommandKind::SetCell,
            Self::SetCells { .. } => LevelCommandKind::SetCells,
            Self::PlaceEntity { .. } => LevelCommandKind::PlaceEntity,
            Self::PlaceEntities { .. } => LevelCommandKind::PlaceEntities,
            Self::MoveEntity { .. } => LevelCommandKind::MoveEntity,
            Self::MoveEntities { .. } => LevelCommandKind::MoveEntities,
            Self::TransformEntities { .. } => LevelCommandKind::TransformEntities,
            Self::RotateEntityClockwise { .. } => LevelCommandKind::RotateEntityClockwise,
            Self::FlipEntityHorizontal { .. } => LevelCommandKind::FlipEntityHorizontal,
            Self::SetBlindResolution { .. } => LevelCommandKind::SetBlindResolution,
            Self::SetBlindResolutions { .. } => LevelCommandKind::SetBlindResolutions,
            Self::ApplyImageToPools { .. } => LevelCommandKind::ApplyImageToPools,
            Self::SetPoolDistributionGroups { .. } => LevelCommandKind::SetPoolDistributionGroups,
            Self::ApplyDistribution { .. } => LevelCommandKind::ApplyDistribution,
            Self::SetBlockCollectLayers { .. } => LevelCommandKind::SetBlockCollectLayers,
            Self::SetBlockLayerCapacity { .. } => LevelCommandKind::SetBlockLayerCapacity,
            Self::DeleteEntity { .. } => LevelCommandKind::DeleteEntity,
            Self::DeleteEntities { .. } => LevelCommandKind::DeleteEntities,
            Self::RestoreEntity { .. } => LevelCommandKind::RestoreEntity,
            Self::RestoreEntities { .. } => LevelCommandKind::RestoreEntities,
            Self::RestoreDeletedEntity { .. } => LevelCommandKind::RestoreDeletedEntity,
            Self::RestoreDeletedEntities { .. } => LevelCommandKind::RestoreDeletedEntities,
            Self::ToggleIce { .. } => LevelCommandKind::ToggleIce,
            Self::SetIce { .. } => LevelCommandKind::SetIce,
            Self::ToggleGlass { .. } => LevelCommandKind::ToggleGlass,
            Self::SetGlass { .. } => LevelCommandKind::SetGlass,
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
    Grid,
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
    Grid(Box<LevelSnapshot>),
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
