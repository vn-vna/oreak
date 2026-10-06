use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::{
    CardinalDirection, CollectCapacity, Decorator, DecoratorId, DecoratorKind, DirectionMode,
    EntityError, EntityId, GridError, GridPoint, GridSize, PlaceableEntity, PlaceableEntityKind,
    ShapeCell,
};

const LEVEL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellKind {
    #[default]
    Floor,
    Wall,
}

impl CellKind {
    const fn hash_tag(self) -> u8 {
        match self {
            Self::Floor => 0,
            Self::Wall => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LevelHash([u8; 32]);

impl LevelHash {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for LevelHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LevelSnapshot {
    schema_version: u32,
    size: GridSize,
    cells: Vec<CellKind>,
    entities: Vec<PlaceableEntity>,
    decorators: Vec<Decorator>,
}

#[derive(Deserialize)]
struct SerializedLevelSnapshot {
    schema_version: u32,
    size: GridSize,
    cells: Vec<CellKind>,
    #[serde(default)]
    entities: Vec<PlaceableEntity>,
    #[serde(default)]
    decorators: Vec<Decorator>,
}

impl<'de> Deserialize<'de> for LevelSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let serialized = SerializedLevelSnapshot::deserialize(deserializer)?;
        if serialized.schema_version != LEVEL_SCHEMA_VERSION {
            return Err(D::Error::custom(format!(
                "unsupported level schema version {}; expected {LEVEL_SCHEMA_VERSION}",
                serialized.schema_version
            )));
        }

        Self::from_parts(
            serialized.size,
            serialized.cells,
            serialized.entities,
            serialized.decorators,
        )
        .map_err(D::Error::custom)
    }
}

impl LevelSnapshot {
    pub fn new(width: u16, height: u16) -> Result<Self, GridError> {
        let size = GridSize::new(width, height)?;
        Ok(Self {
            schema_version: LEVEL_SCHEMA_VERSION,
            size,
            cells: vec![CellKind::Floor; size.cell_count()],
            entities: Vec::new(),
            decorators: Vec::new(),
        })
    }

    pub fn from_cells(size: GridSize, cells: Vec<CellKind>) -> Result<Self, GridError> {
        validate_cells(size, &cells)?;
        Ok(Self {
            schema_version: LEVEL_SCHEMA_VERSION,
            size,
            cells,
            entities: Vec::new(),
            decorators: Vec::new(),
        })
    }

    pub fn from_parts(
        size: GridSize,
        cells: Vec<CellKind>,
        mut entities: Vec<PlaceableEntity>,
        mut decorators: Vec<Decorator>,
    ) -> Result<Self, LevelError> {
        validate_cells(size, &cells)?;
        entities.sort_by(|left, right| left.id().cmp(right.id()));
        decorators.sort_by(|left, right| left.id().cmp(right.id()));

        let snapshot = Self {
            schema_version: LEVEL_SCHEMA_VERSION,
            size,
            cells,
            entities,
            decorators,
        };
        snapshot.validate_domain()?;
        Ok(snapshot)
    }

    /// Retains the original grid-oriented validation API while checking the full snapshot.
    pub fn validate(&self) -> Result<(), GridError> {
        self.validate_domain().map_err(|error| match error {
            LevelError::InvalidGrid(error) => error,
            error => GridError::InvalidSnapshot {
                reason: error.to_string(),
            },
        })
    }

    pub fn validate_domain(&self) -> Result<(), LevelError> {
        validate_cells(self.size, &self.cells)?;

        for entities in self.entities.windows(2) {
            if entities[0].id() == entities[1].id() {
                return Err(LevelError::DuplicateEntityId(entities[0].id().clone()));
            }
        }
        for (index, entity) in self.entities.iter().enumerate() {
            self.validate_entity_placement(entity, Some(index))?;
        }

        self.validate_decorators(&self.decorators)?;

        Ok(())
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn size(&self) -> GridSize {
        self.size
    }

    #[must_use]
    pub fn cells(&self) -> &[CellKind] {
        &self.cells
    }

    #[must_use]
    pub fn entities(&self) -> &[PlaceableEntity] {
        &self.entities
    }

    #[must_use]
    pub fn decorators(&self) -> &[Decorator] {
        &self.decorators
    }

    #[must_use]
    pub fn decorator(&self, id: &DecoratorId) -> Option<&Decorator> {
        self.decorator_index(id)
            .ok()
            .map(|index| &self.decorators[index])
    }

    #[must_use]
    pub fn ice_for_entity(&self, entity_id: &EntityId) -> Option<&Decorator> {
        self.decorators.iter().find(|decorator| {
            matches!(
                decorator.kind(),
                DecoratorKind::Ice { entity, .. } if entity == entity_id
            )
        })
    }

    #[must_use]
    pub fn ice_blocking_count(&self, entity_id: &EntityId) -> u32 {
        self.ice_for_entity(entity_id)
            .and_then(Decorator::ice_blocking_count)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn glass_for_entity(&self, entity_id: &EntityId) -> Option<&Decorator> {
        self.decorators.iter().find(|decorator| {
            matches!(
                decorator.kind(),
                DecoratorKind::Glass { entity, .. } if entity == entity_id
            )
        })
    }

    #[must_use]
    pub fn glass_blocking_count(&self, entity_id: &EntityId) -> u32 {
        self.glass_for_entity(entity_id)
            .and_then(Decorator::glass_blocking_count)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn direction_for_entity(&self, entity_id: &EntityId) -> Option<&Decorator> {
        self.decorators.iter().find(|decorator| {
            matches!(
                decorator.kind(),
                DecoratorKind::Direction { entity, .. } if entity == entity_id
            )
        })
    }

    #[must_use]
    pub fn direction_mode(&self, entity_id: &EntityId) -> DirectionMode {
        self.direction_for_entity(entity_id)
            .and_then(Decorator::direction_constraint)
            .unwrap_or_default()
    }

    pub fn cell(&self, point: GridPoint) -> Result<CellKind, GridError> {
        Ok(self.cells[self.size.index_of(point)?])
    }

    #[must_use]
    pub fn entity(&self, id: &EntityId) -> Option<&PlaceableEntity> {
        self.entity_index(id)
            .ok()
            .map(|index| &self.entities[index])
    }

    #[must_use]
    pub fn entity_at(&self, point: GridPoint) -> Option<&PlaceableEntity> {
        self.entities.iter().find(|entity| {
            entity
                .shape()
                .occupied_cells()
                .any(|cell| world_point(entity.origin(), cell).is_some_and(|world| world == point))
        })
    }

    pub(crate) fn set_cell(&mut self, point: GridPoint, kind: CellKind) -> Result<bool, GridError> {
        let index = self.size.index_of(point)?;
        if self.cells[index] == kind {
            return Ok(false);
        }

        self.cells[index] = kind;
        Ok(true)
    }

    pub(crate) fn insert_entity(&mut self, entity: PlaceableEntity) -> Result<(), LevelError> {
        let index = match self.entity_index(entity.id()) {
            Ok(_) => return Err(LevelError::DuplicateEntityId(entity.id().clone())),
            Err(index) => index,
        };
        self.validate_entity_placement(&entity, None)?;
        self.entities.insert(index, entity);
        Ok(())
    }

    pub(crate) fn replace_entity(
        &mut self,
        entity: PlaceableEntity,
    ) -> Result<PlaceableEntity, LevelError> {
        let index = self
            .entity_index(entity.id())
            .map_err(|_| LevelError::EntityNotFound(entity.id().clone()))?;
        self.validate_entity_placement(&entity, Some(index))?;
        Ok(std::mem::replace(&mut self.entities[index], entity))
    }

    pub(crate) fn replace_entities(
        &mut self,
        replacements: Vec<PlaceableEntity>,
    ) -> Result<(), LevelError> {
        let mut ids = BTreeSet::new();
        let mut candidate = self.entities.clone();
        for entity in replacements {
            if !ids.insert(entity.id().clone()) {
                return Err(LevelError::DuplicateEntityMove(entity.id().clone()));
            }
            let index = candidate
                .binary_search_by(|existing| existing.id().cmp(entity.id()))
                .map_err(|_| LevelError::EntityNotFound(entity.id().clone()))?;
            candidate[index] = entity;
        }
        let previous = std::mem::replace(&mut self.entities, candidate);
        if let Err(error) = self.validate_domain() {
            self.entities = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn remove_entity_cascade(
        &mut self,
        id: &EntityId,
    ) -> Result<(PlaceableEntity, Vec<Decorator>), LevelError> {
        let index = self
            .entity_index(id)
            .map_err(|_| LevelError::EntityNotFound(id.clone()))?;
        let removed_decorators: Vec<_> = self
            .decorators
            .iter()
            .filter(|decorator| decorator.references(id))
            .cloned()
            .collect();
        self.decorators
            .retain(|decorator| !decorator.references(id));
        Ok((self.entities.remove(index), removed_decorators))
    }

    pub(crate) fn restore_entity_cascade(
        &mut self,
        entity: PlaceableEntity,
        decorators: &[Decorator],
    ) -> Result<(), LevelError> {
        self.insert_entity(entity)?;
        for decorator in decorators {
            if self.decorator(decorator.id()).is_some() {
                return Err(LevelError::DuplicateDecoratorId(decorator.id().clone()));
            }
            self.put_decorator(decorator.clone())?;
        }
        Ok(())
    }

    pub(crate) fn put_decorator(
        &mut self,
        decorator: Decorator,
    ) -> Result<Option<Decorator>, LevelError> {
        let mut candidate = self.decorators.clone();
        let before = match candidate.binary_search_by(|item| item.id().cmp(decorator.id())) {
            Ok(index) => Some(std::mem::replace(&mut candidate[index], decorator)),
            Err(index) => {
                candidate.insert(index, decorator);
                None
            }
        };
        self.validate_decorators(&candidate)?;
        self.decorators = candidate;
        Ok(before)
    }

    pub(crate) fn remove_decorator(&mut self, id: &DecoratorId) -> Option<Decorator> {
        let index = self.decorator_index(id).ok()?;
        Some(self.decorators.remove(index))
    }

    #[must_use]
    pub fn content_hash(&self) -> LevelHash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"oreak-level-snapshot");
        hasher.update(&self.schema_version.to_le_bytes());
        hasher.update(&self.size.width().to_le_bytes());
        hasher.update(&self.size.height().to_le_bytes());
        for cell in &self.cells {
            hasher.update(&[cell.hash_tag()]);
        }

        if self.entities.is_empty() && self.decorators.is_empty() {
            return LevelHash(*hasher.finalize().as_bytes());
        }

        hasher.update(b"oreak-level-domain-v1");
        hash_len(&mut hasher, self.entities.len());
        for entity in &self.entities {
            hash_string(&mut hasher, entity.id().as_str());
            hasher.update(&entity.origin().x.to_le_bytes());
            hasher.update(&entity.origin().y.to_le_bytes());
            hasher.update(&[entity.shape().width(), entity.shape().height()]);
            hasher.update(&entity.shape().occupied_mask().to_le_bytes());
            match entity.kind() {
                PlaceableEntityKind::Block(block) => {
                    hasher.update(&[0]);
                    hash_len(&mut hasher, block.collect_layers().len());
                    for layer in block.collect_layers() {
                        hasher.update(&layer.color_index().to_le_bytes());
                        match layer.radius() {
                            Some(radius) => hasher.update(&[1, radius]),
                            None => hasher.update(&[0]),
                        };
                        match layer.capacity() {
                            CollectCapacity::Unlimited => hasher.update(&[0]),
                            CollectCapacity::Finite(capacity) => {
                                hasher.update(&[1]);
                                hasher.update(&capacity.to_le_bytes())
                            }
                        };
                        hasher.update(&[u8::from(layer.is_locked())]);
                    }
                }
                PlaceableEntityKind::Blind(blind) => {
                    hasher.update(&[1, blind.pixels_per_cell()]);
                    hash_len(&mut hasher, blind.tiles().len());
                    for tile in blind.tiles() {
                        hash_len(&mut hasher, tile.bits().len());
                        hasher.update(tile.bits());
                        if tile.has_non_default_colors() {
                            hasher.update(b"oreak-blind-colors");
                            hash_len(&mut hasher, tile.colors().len());
                            hasher.update(tile.colors());
                        }
                    }
                    if !blind.guides().is_empty() {
                        hasher.update(b"oreak-blind-guides");
                        hash_len(&mut hasher, blind.guides().len());
                        for guide in blind.guides() {
                            hasher.update(&guide.first().x.to_le_bytes());
                            hasher.update(&guide.first().y.to_le_bytes());
                            hasher.update(&guide.second().x.to_le_bytes());
                            hasher.update(&guide.second().y.to_le_bytes());
                        }
                    }
                }
            }
        }

        hash_len(&mut hasher, self.decorators.len());
        for decorator in &self.decorators {
            hash_string(&mut hasher, decorator.id().as_str());
            match decorator.kind() {
                DecoratorKind::Ice {
                    entity,
                    blocking_count,
                } => {
                    hasher.update(&[0]);
                    hash_string(&mut hasher, entity.as_str());
                    if *blocking_count != 1 {
                        hasher.update(b"oreak-ice-blocking-count");
                        hasher.update(&blocking_count.to_le_bytes());
                    }
                }
                DecoratorKind::Glass {
                    entity,
                    blocking_count,
                } => {
                    hasher.update(&[3]);
                    hash_string(&mut hasher, entity.as_str());
                    if *blocking_count != 1 {
                        hasher.update(b"oreak-glass-blocking-count");
                        hasher.update(&blocking_count.to_le_bytes());
                    }
                }
                DecoratorKind::Direction { entity, direction } => {
                    hasher.update(&[1]);
                    hash_string(&mut hasher, entity.as_str());
                    hasher.update(&[match direction {
                        CardinalDirection::Up => 0,
                        CardinalDirection::Right => 1,
                        CardinalDirection::Down => 2,
                        CardinalDirection::Left => 3,
                    }]);
                }
                DecoratorKind::KeyLocker { entity, key } => {
                    hasher.update(&[2]);
                    hash_string(&mut hasher, entity.as_str());
                    hash_string(&mut hasher, key.as_str());
                }
            }
        }

        LevelHash(*hasher.finalize().as_bytes())
    }

    fn entity_index(&self, id: &EntityId) -> Result<usize, usize> {
        self.entities.binary_search_by(|entity| entity.id().cmp(id))
    }

    fn decorator_index(&self, id: &DecoratorId) -> Result<usize, usize> {
        self.decorators
            .binary_search_by(|decorator| decorator.id().cmp(id))
    }

    fn validate_decorators(&self, decorators: &[Decorator]) -> Result<(), LevelError> {
        for pair in decorators.windows(2) {
            if pair[0].id() == pair[1].id() {
                return Err(LevelError::DuplicateDecoratorId(pair[0].id().clone()));
            }
        }

        let mut ice_owners = BTreeMap::new();
        let mut glass_owners = BTreeMap::new();
        let mut direction_owners = BTreeMap::new();
        let mut key_owners = BTreeMap::new();
        let mut lock_owners = BTreeMap::new();
        for decorator in decorators {
            for entity_id in decorator.referenced_entities() {
                if self.entity(entity_id).is_none() {
                    return Err(LevelError::MissingDecoratorEntity {
                        decorator_id: decorator.id().clone(),
                        entity_id: entity_id.clone(),
                    });
                }
            }

            match decorator.kind() {
                DecoratorKind::Ice { entity, .. } => {
                    self.validate_decorator_block_owner(decorator.id(), entity)?;
                    insert_owner(&mut ice_owners, DecoratorRole::Ice, entity, decorator.id())?
                }
                DecoratorKind::Glass { entity, .. } => {
                    self.validate_decorator_blind_owner(decorator.id(), entity)?;
                    insert_owner(
                        &mut glass_owners,
                        DecoratorRole::Glass,
                        entity,
                        decorator.id(),
                    )?
                }
                DecoratorKind::Direction { entity, .. } => {
                    self.validate_decorator_block_owner(decorator.id(), entity)?;
                    insert_owner(
                        &mut direction_owners,
                        DecoratorRole::Direction,
                        entity,
                        decorator.id(),
                    )?
                }
                DecoratorKind::KeyLocker { entity, key } => {
                    if entity == key {
                        return Err(LevelError::KeyLockerEndpointsMustDiffer {
                            decorator_id: decorator.id().clone(),
                            entity_id: entity.clone(),
                        });
                    }
                    for endpoint in [entity, key] {
                        let is_block = self.entity(endpoint).is_some_and(|entity| {
                            matches!(entity.kind(), PlaceableEntityKind::Block(_))
                        });
                        if !is_block {
                            return Err(LevelError::KeyLockerEndpointNotBlock {
                                decorator_id: decorator.id().clone(),
                                entity_id: endpoint.clone(),
                            });
                        }
                    }
                    insert_owner(
                        &mut lock_owners,
                        DecoratorRole::Lock,
                        entity,
                        decorator.id(),
                    )?;
                    insert_owner(&mut key_owners, DecoratorRole::Key, key, decorator.id())?;
                }
            }
        }
        Ok(())
    }

    fn validate_decorator_block_owner(
        &self,
        decorator_id: &DecoratorId,
        entity_id: &EntityId,
    ) -> Result<(), LevelError> {
        if self
            .entity(entity_id)
            .is_some_and(|entity| matches!(entity.kind(), PlaceableEntityKind::Block(_)))
        {
            return Ok(());
        }
        Err(LevelError::DecoratorOwnerNotBlock {
            decorator_id: decorator_id.clone(),
            entity_id: entity_id.clone(),
        })
    }

    fn validate_decorator_blind_owner(
        &self,
        decorator_id: &DecoratorId,
        entity_id: &EntityId,
    ) -> Result<(), LevelError> {
        if self
            .entity(entity_id)
            .is_some_and(|entity| matches!(entity.kind(), PlaceableEntityKind::Blind(_)))
        {
            return Ok(());
        }
        Err(LevelError::DecoratorOwnerNotBlind {
            decorator_id: decorator_id.clone(),
            entity_id: entity_id.clone(),
        })
    }

    fn validate_entity_placement(
        &self,
        entity: &PlaceableEntity,
        ignored_index: Option<usize>,
    ) -> Result<(), LevelError> {
        for local_cell in entity.shape().occupied_cells() {
            let Some(point) = world_point(entity.origin(), local_cell) else {
                return Err(LevelError::EntityOutOfBounds {
                    entity_id: entity.id().clone(),
                    origin: entity.origin(),
                    local_cell,
                    size: self.size,
                });
            };
            let Ok(index) = self.size.index_of(point) else {
                return Err(LevelError::EntityOutOfBounds {
                    entity_id: entity.id().clone(),
                    origin: entity.origin(),
                    local_cell,
                    size: self.size,
                });
            };
            if self.cells[index] == CellKind::Wall {
                return Err(LevelError::EntityOnWall {
                    entity_id: entity.id().clone(),
                    point,
                });
            }

            for (index, other) in self.entities.iter().enumerate() {
                if Some(index) == ignored_index {
                    continue;
                }
                let overlaps = other.shape().occupied_cells().any(|other_cell| {
                    world_point(other.origin(), other_cell).is_some_and(|world| world == point)
                });
                if overlaps {
                    return Err(LevelError::EntityOverlap {
                        entity_id: entity.id().clone(),
                        other_entity_id: other.id().clone(),
                        point,
                    });
                }
            }
        }

        Ok(())
    }
}

fn insert_owner(
    owners: &mut BTreeMap<EntityId, DecoratorId>,
    role: DecoratorRole,
    entity_id: &EntityId,
    decorator_id: &DecoratorId,
) -> Result<(), LevelError> {
    if let Some(first_decorator_id) = owners.insert(entity_id.clone(), decorator_id.clone()) {
        return Err(LevelError::DecoratorOwnershipConflict {
            role,
            entity_id: entity_id.clone(),
            first_decorator_id,
            second_decorator_id: decorator_id.clone(),
        });
    }
    Ok(())
}

fn validate_cells(size: GridSize, cells: &[CellKind]) -> Result<(), GridError> {
    size.validate()?;
    if cells.len() != size.cell_count() {
        return Err(GridError::CellCountMismatch {
            expected: size.cell_count(),
            actual: cells.len(),
        });
    }
    Ok(())
}

fn world_point(origin: GridPoint, cell: ShapeCell) -> Option<GridPoint> {
    Some(GridPoint::new(
        origin.x.checked_add(u16::from(cell.x))?,
        origin.y.checked_add(u16::from(cell.y))?,
    ))
}

fn hash_len(hasher: &mut blake3::Hasher, len: usize) {
    hasher.update(&(len as u64).to_le_bytes());
}

fn hash_string(hasher: &mut blake3::Hasher, value: &str) {
    hash_len(hasher, value.len());
    hasher.update(value.as_bytes());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecoratorRole {
    Ice,
    Glass,
    Direction,
    Key,
    Lock,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LevelError {
    #[error(transparent)]
    InvalidGrid(#[from] GridError),

    #[error(transparent)]
    InvalidEntity(#[from] EntityError),

    #[error("entity ID '{0}' occurs more than once")]
    DuplicateEntityId(EntityId),

    #[error("entity ID '{0}' does not exist")]
    EntityNotFound(EntityId),

    #[error("entity ID '{0}' occurs more than once in a grouped move")]
    DuplicateEntityMove(EntityId),

    #[error(
        "entity '{entity_id}' at ({}, {}) with local cell ({}, {}) is outside the {} x {} grid",
        origin.x,
        origin.y,
        local_cell.x,
        local_cell.y,
        size.width(),
        size.height()
    )]
    EntityOutOfBounds {
        entity_id: EntityId,
        origin: GridPoint,
        local_cell: ShapeCell,
        size: GridSize,
    },

    #[error("entity '{entity_id}' occupies wall cell ({}, {})", point.x, point.y)]
    EntityOnWall {
        entity_id: EntityId,
        point: GridPoint,
    },

    #[error(
        "entity '{entity_id}' overlaps entity '{other_entity_id}' at ({}, {})",
        point.x,
        point.y
    )]
    EntityOverlap {
        entity_id: EntityId,
        other_entity_id: EntityId,
        point: GridPoint,
    },

    #[error("cell ({}, {}) is occupied by entity '{entity_id}'", point.x, point.y)]
    CellOccupiedByEntity {
        point: GridPoint,
        entity_id: EntityId,
    },

    #[error("decorator ID '{0}' occurs more than once")]
    DuplicateDecoratorId(DecoratorId),

    #[error("decorator '{decorator_id}' references missing entity '{entity_id}'")]
    MissingDecoratorEntity {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },

    #[error(
        "decorators '{first_decorator_id}' and '{second_decorator_id}' both own {role:?} for entity '{entity_id}'"
    )]
    DecoratorOwnershipConflict {
        role: DecoratorRole,
        entity_id: EntityId,
        first_decorator_id: DecoratorId,
        second_decorator_id: DecoratorId,
    },

    #[error("KeyLocker decorator '{decorator_id}' uses entity '{entity_id}' as both endpoints")]
    KeyLockerEndpointsMustDiffer {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },

    #[error("KeyLocker decorator '{decorator_id}' endpoint '{entity_id}' is not a Block")]
    KeyLockerEndpointNotBlock {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },

    #[error("decorator '{decorator_id}' owner '{entity_id}' is not a Block")]
    DecoratorOwnerNotBlock {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },

    #[error("decorator '{decorator_id}' owner '{entity_id}' is not a Blind")]
    DecoratorOwnerNotBlind {
        decorator_id: DecoratorId,
        entity_id: EntityId,
    },

    #[error("decorator '{decorator_id}' has a different kind or owner than the command")]
    DecoratorIdentityConflict { decorator_id: DecoratorId },

    #[error("entity '{entity_id}' is referenced by decorator '{decorator_id}'")]
    EntityReferencedByDecorator {
        entity_id: EntityId,
        decorator_id: DecoratorId,
    },

    #[error("resizing would clip non-floor cell ({}, {})", point.x, point.y)]
    ResizeWouldClipCell { point: GridPoint },

    #[error("resizing would clip entity '{entity_id}'")]
    ResizeWouldClipEntity { entity_id: EntityId },
}
