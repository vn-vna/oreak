//! Deterministic level behavior shared by Oreak's server and WebAssembly client.
//!
//! Platform concerns such as clocks, random IDs, storage, networking, and UI
//! are deliberately kept outside this crate.

mod brush;
mod command;
mod decorator;
pub mod distribution;
mod entity;
mod grid;
mod ids;
pub mod image_art;
mod level;
mod pool_boundary;
mod sandbox;
mod shape;
mod timeline;

pub use brush::{
    BlindBrushOperation, BlindGuide, BlindGuidePatch, BlindGuideSet, BlindPixel, BlindStroke,
    BlindTilePatch, BrushError, MAX_BLIND_BRUSH_PIXELS, MAX_BLIND_COLOR_INDEX, MAX_BLIND_GUIDES,
    MIN_BLIND_COLOR_INDEX,
};
pub use command::{
    CellEdit, CommandEnvelope, CommandMetadata, EntityMove, EntityTransform, GridAnchor,
    HistoryChange, HistoryEvent, LevelCommand, LevelCommandKind, LevelTarget, LevelValue,
};
pub use decorator::{CardinalDirection, Decorator, DecoratorKind, DirectionMode};
pub use distribution::{
    DistributionColorSummary, DistributionError, DistributionLayerAllocation,
    DistributionLayerSelection, DistributionPlan, DistributionPoolSelection, DistributionRequest,
    distribution_plan, pool_group_counts, pool_all_group_counts,
};
pub use entity::{
    Blind, BlindTile, Block, CollectCapacity, CollectLayer, DISABLED_COLLECT_COLOR_INDEX,
    EntityError, MAX_BLIND_PIXELS_PER_CELL, MAX_COLLECT_CAPACITY, MAX_COLLECT_COLOR_INDEX,
    MAX_COLLECT_LAYERS, MAX_POOL_DISTRIBUTION_GROUP_NAME_CHARS, MAX_POOL_DISTRIBUTION_GROUPS,
    MIN_BLIND_PIXELS_PER_CELL, PlaceableEntity, PlaceableEntityKind, PoolDistributionGroup,
};
pub use grid::{GridError, GridPoint, GridSize, MAX_GRID_AXIS};
pub use ids::{ActorId, CommandId, DecoratorId, EntityId};
pub use image_art::{
    DitherMode, ImageArtError, ImagePlacement, ImageProjector, ImageSampling, ImageTransparency,
    IndexedImage, MAX_IMAGE_AXIS, MAX_PALETTE_MAPPINGS, PALETTE_HEX, PALETTE_RGB, PALETTE_VERSION,
    PaletteMapping, PaletteSettings, SourceColorCluster, convert_rgba, extract_color_clusters,
    project_image,
};
pub use level::{CellKind, DecoratorRole, LevelError, LevelHash, LevelSnapshot};
pub use pool_boundary::{PoolBoundary, PoolBoundaryError, PoolPlayableMask, pool_playable_mask};
pub use sandbox::{
    SANDBOX_SAND_STEPPING_SUPPORTED, SandboxError, SandboxMoveDirection, SandboxSession,
};
pub use shape::{MAX_BLOCK_SHAPE_AXIS, MAX_SHAPE_AREA, Shape, ShapeCell, ShapeError};
pub use timeline::{ApplyOutcome, BlameEntry, LevelTimeline, TimelineError};
