use serde::{Deserialize, Serialize};

use crate::{DecoratorId, EntityId};

const fn default_ice_blocking_count() -> u32 {
    1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardinalDirection {
    Up,
    Right,
    Down,
    Left,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectionMode {
    #[default]
    Disabled,
    Horizontal,
    Vertical,
}

impl CardinalDirection {
    #[must_use]
    pub const fn mode(self) -> DirectionMode {
        match self {
            Self::Left | Self::Right => DirectionMode::Horizontal,
            Self::Up | Self::Down => DirectionMode::Vertical,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecoratorKind {
    Ice {
        entity: EntityId,
        #[serde(default = "default_ice_blocking_count")]
        blocking_count: u32,
    },
    Direction {
        entity: EntityId,
        direction: CardinalDirection,
    },
    KeyLocker {
        entity: EntityId,
        key: EntityId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decorator {
    id: DecoratorId,
    kind: DecoratorKind,
}

impl Decorator {
    #[must_use]
    pub fn ice(id: impl Into<DecoratorId>, entity: impl Into<EntityId>) -> Self {
        Self::ice_with_blocking_count(id, entity, default_ice_blocking_count())
    }

    #[must_use]
    pub fn ice_with_blocking_count(
        id: impl Into<DecoratorId>,
        entity: impl Into<EntityId>,
        blocking_count: u32,
    ) -> Self {
        Self {
            id: id.into(),
            kind: DecoratorKind::Ice {
                entity: entity.into(),
                blocking_count,
            },
        }
    }

    #[must_use]
    pub fn direction(
        id: impl Into<DecoratorId>,
        entity: impl Into<EntityId>,
        direction: CardinalDirection,
    ) -> Self {
        Self {
            id: id.into(),
            kind: DecoratorKind::Direction {
                entity: entity.into(),
                direction,
            },
        }
    }

    #[must_use]
    pub fn direction_mode(
        id: impl Into<DecoratorId>,
        entity: impl Into<EntityId>,
        mode: DirectionMode,
    ) -> Option<Self> {
        let direction = match mode {
            DirectionMode::Disabled => return None,
            DirectionMode::Horizontal => CardinalDirection::Right,
            DirectionMode::Vertical => CardinalDirection::Up,
        };
        Some(Self::direction(id, entity, direction))
    }

    /// `entity` is the lock endpoint and `key` is the key endpoint.
    #[must_use]
    pub fn key_locker(
        id: impl Into<DecoratorId>,
        entity: impl Into<EntityId>,
        key: impl Into<EntityId>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: DecoratorKind::KeyLocker {
                entity: entity.into(),
                key: key.into(),
            },
        }
    }

    #[must_use]
    pub const fn id(&self) -> &DecoratorId {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> &DecoratorKind {
        &self.kind
    }

    #[must_use]
    pub const fn ice_blocking_count(&self) -> Option<u32> {
        match &self.kind {
            DecoratorKind::Ice { blocking_count, .. } => Some(*blocking_count),
            _ => None,
        }
    }

    #[must_use]
    pub const fn direction_constraint(&self) -> Option<DirectionMode> {
        match &self.kind {
            DecoratorKind::Direction { direction, .. } => Some(direction.mode()),
            _ => None,
        }
    }

    #[must_use]
    pub fn references(&self, entity_id: &EntityId) -> bool {
        match &self.kind {
            DecoratorKind::Ice { entity, .. } | DecoratorKind::Direction { entity, .. } => {
                entity == entity_id
            }
            DecoratorKind::KeyLocker { entity, key } => entity == entity_id || key == entity_id,
        }
    }

    pub(crate) fn referenced_entities(&self) -> impl Iterator<Item = &EntityId> {
        let (first, second) = match &self.kind {
            DecoratorKind::Ice { entity, .. } | DecoratorKind::Direction { entity, .. } => {
                (entity, None)
            }
            DecoratorKind::KeyLocker { entity, key } => (entity, Some(key)),
        };
        std::iter::once(first).chain(second)
    }
}
