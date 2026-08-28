//! Deterministic level behavior shared by Oreak's server and WebAssembly client.
//!
//! Platform concerns such as clocks, random IDs, storage, networking, and UI
//! are deliberately kept outside this crate.

mod brush;
mod command;
mod decorator;
mod entity;
mod grid;
mod ids;
mod level;
mod sandbox;
mod shape;
mod timeline;

pub use brush::{
    BlindBrushOperation, BlindGuide, BlindGuidePatch, BlindGuideSet, BlindPixel, BlindStroke,
    BlindTilePatch, BrushError, MAX_BLIND_BRUSH_PIXELS, MAX_BLIND_COLOR_INDEX, MAX_BLIND_GUIDES,
    MIN_BLIND_COLOR_INDEX,
};
pub use command::{
    CommandEnvelope, CommandMetadata, EntityMove, GridAnchor, HistoryChange, HistoryEvent,
    LevelCommand, LevelCommandKind, LevelTarget, LevelValue,
};
pub use decorator::{CardinalDirection, Decorator, DecoratorKind, DirectionMode};
pub use entity::{
    Blind, BlindTile, Block, CollectCapacity, CollectLayer, EntityError, MAX_BLIND_PIXELS_PER_CELL,
    MIN_BLIND_PIXELS_PER_CELL, PlaceableEntity, PlaceableEntityKind,
};
pub use grid::{GridError, GridPoint, GridSize, MAX_GRID_AXIS};
pub use ids::{ActorId, CommandId, DecoratorId, EntityId};
pub use level::{CellKind, DecoratorRole, LevelError, LevelHash, LevelSnapshot};
pub use sandbox::{
    SANDBOX_SAND_STEPPING_SUPPORTED, SandboxError, SandboxMoveDirection, SandboxSession,
};
pub use shape::{MAX_BLOCK_SHAPE_AXIS, MAX_SHAPE_AREA, Shape, ShapeCell, ShapeError};
pub use timeline::{ApplyOutcome, BlameEntry, LevelTimeline, TimelineError};
