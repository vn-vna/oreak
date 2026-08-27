use thiserror::Error;

use crate::{
    ApplyOutcome, CommandEnvelope, CommandMetadata, DirectionMode, EntityId, GridPoint,
    HistoryEvent, LevelCommand, LevelError, LevelSnapshot, LevelTimeline, TimelineError,
};

pub const SANDBOX_SAND_STEPPING_SUPPORTED: bool = false;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxMoveDirection {
    Up,
    Right,
    Down,
    Left,
}

/// An isolated deterministic play copy. Sand stepping is intentionally deferred.
#[derive(Clone, Debug)]
pub struct SandboxSession {
    authored: LevelSnapshot,
    timeline: LevelTimeline,
    next_command: u64,
}

impl SandboxSession {
    pub fn new(authored: &LevelSnapshot) -> Result<Self, SandboxError> {
        let authored = authored.clone();
        let timeline = LevelTimeline::new(authored.clone())?;
        Ok(Self {
            authored,
            timeline,
            next_command: 1,
        })
    }

    #[must_use]
    pub const fn authored_snapshot(&self) -> &LevelSnapshot {
        &self.authored
    }

    #[must_use]
    pub const fn snapshot(&self) -> &LevelSnapshot {
        self.timeline.snapshot()
    }

    #[must_use]
    pub const fn timeline(&self) -> &LevelTimeline {
        &self.timeline
    }

    #[must_use]
    pub fn history(&self) -> &[HistoryEvent] {
        self.timeline.events()
    }

    pub fn restart(&mut self) -> Result<(), SandboxError> {
        self.timeline = LevelTimeline::new(self.authored.clone())?;
        self.next_command = 1;
        Ok(())
    }

    pub fn move_entity(
        &mut self,
        entity_id: &EntityId,
        direction: SandboxMoveDirection,
    ) -> Result<ApplyOutcome, SandboxError> {
        let entity = self
            .timeline
            .snapshot()
            .entity(entity_id)
            .ok_or_else(|| LevelError::EntityNotFound(entity_id.clone()))?;
        let blocking_count = self.timeline.snapshot().ice_blocking_count(entity_id);
        if blocking_count > 0 {
            return Err(SandboxError::BlockedByIce {
                entity_id: entity_id.clone(),
                blocking_count,
            });
        }

        let mode = self.timeline.snapshot().direction_mode(entity_id);
        let allowed = match mode {
            DirectionMode::Disabled => true,
            DirectionMode::Horizontal => matches!(
                direction,
                SandboxMoveDirection::Left | SandboxMoveDirection::Right
            ),
            DirectionMode::Vertical => matches!(
                direction,
                SandboxMoveDirection::Up | SandboxMoveDirection::Down
            ),
        };
        if !allowed {
            return Err(SandboxError::BlockedByDirection {
                entity_id: entity_id.clone(),
                mode,
                attempted: direction,
            });
        }

        let origin = moved_origin(entity.origin(), direction).ok_or_else(|| {
            SandboxError::MovementOutOfBounds {
                entity_id: entity_id.clone(),
                direction,
            }
        })?;
        let next_command = self
            .next_command
            .checked_add(1)
            .ok_or(SandboxError::CommandSequenceExhausted)?;
        let metadata =
            CommandMetadata::new(format!("sandbox-move-{}", self.next_command), "sandbox", 0);
        let outcome = self.timeline.apply(CommandEnvelope::new(
            metadata,
            LevelCommand::MoveEntity {
                entity_id: entity_id.clone(),
                origin,
            },
        ))?;
        self.next_command = next_command;
        Ok(outcome)
    }

    /// Sand simulation is not part of this parity layer yet.
    pub const fn step_sand(&mut self) -> Result<(), SandboxError> {
        Err(SandboxError::SandSteppingDeferred)
    }
}

fn moved_origin(origin: GridPoint, direction: SandboxMoveDirection) -> Option<GridPoint> {
    match direction {
        SandboxMoveDirection::Up => Some(GridPoint::new(origin.x, origin.y.checked_add(1)?)),
        SandboxMoveDirection::Right => Some(GridPoint::new(origin.x.checked_add(1)?, origin.y)),
        SandboxMoveDirection::Down => Some(GridPoint::new(origin.x, origin.y.checked_sub(1)?)),
        SandboxMoveDirection::Left => Some(GridPoint::new(origin.x.checked_sub(1)?, origin.y)),
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    #[error(transparent)]
    Timeline(#[from] TimelineError),

    #[error(transparent)]
    InvalidLevel(#[from] LevelError),

    #[error("entity '{entity_id}' is blocked by {blocking_count} Ice layer(s)")]
    BlockedByIce {
        entity_id: EntityId,
        blocking_count: u32,
    },

    #[error("entity '{entity_id}' is constrained to {mode:?}, blocking {attempted:?}")]
    BlockedByDirection {
        entity_id: EntityId,
        mode: DirectionMode,
        attempted: SandboxMoveDirection,
    },

    #[error("entity '{entity_id}' cannot move {direction:?} outside unsigned map coordinates")]
    MovementOutOfBounds {
        entity_id: EntityId,
        direction: SandboxMoveDirection,
    },

    #[error("the Sandbox command sequence is exhausted")]
    CommandSequenceExhausted,

    #[error("sand stepping is explicitly deferred in SandboxSession")]
    SandSteppingDeferred,
}
