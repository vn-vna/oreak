use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ActorId, BlindBrushOperation, BlindGuide, BlindGuidePatch, BlindTilePatch, CellKind,
    CollectCapacity, CollectLayer, CommandEnvelope, CommandId, CommandMetadata, Decorator,
    DecoratorId, DecoratorKind, DirectionMode, EntityError, EntityId, EntityMove, GridAnchor,
    GridError, GridPoint, GridSize, HistoryChange, HistoryEvent, LevelCommand, LevelCommandKind,
    LevelError, LevelHash, LevelSnapshot, LevelTarget, LevelValue, PlaceableEntity,
};

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // Boxing Applied would break the existing public API.
pub enum ApplyOutcome {
    Applied(HistoryEvent),
    NoChange { snapshot_hash: LevelHash },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameEntry {
    pub sequence: u64,
    pub command_id: CommandId,
    pub actor: ActorId,
    pub occurred_at_ms: i64,
    pub command_kind: LevelCommandKind,
    pub reverts_sequence: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct LevelTimeline {
    current: LevelSnapshot,
    events: Vec<HistoryEvent>,
    command_sequences: BTreeMap<CommandId, u64>,
    provenance: BTreeMap<LevelTarget, u64>,
    reverted_sequences: BTreeSet<u64>,
    next_sequence: u64,
}

impl LevelTimeline {
    pub fn new(snapshot: LevelSnapshot) -> Result<Self, TimelineError> {
        snapshot.validate().map_err(TimelineError::InvalidGrid)?;
        Ok(Self {
            current: snapshot,
            events: Vec::new(),
            command_sequences: BTreeMap::new(),
            provenance: BTreeMap::new(),
            reverted_sequences: BTreeSet::new(),
            next_sequence: 1,
        })
    }

    #[must_use]
    pub const fn snapshot(&self) -> &LevelSnapshot {
        &self.current
    }

    #[must_use]
    pub fn events(&self) -> &[HistoryEvent] {
        &self.events
    }

    pub fn apply(&mut self, envelope: CommandEnvelope) -> Result<ApplyOutcome, TimelineError> {
        self.apply_internal(envelope, None)
    }

    pub fn undo_latest(
        &mut self,
        metadata: CommandMetadata,
    ) -> Result<HistoryEvent, TimelineError> {
        if self.command_sequences.contains_key(&metadata.id) {
            return Err(TimelineError::DuplicateCommandId(metadata.id));
        }

        let mut blocked_sequences = Vec::new();
        let target_event = self
            .events
            .iter()
            .rev()
            .filter(|event| {
                event.metadata.actor == metadata.actor
                    && event.reverts_sequence.is_none()
                    && !self.reverted_sequences.contains(&event.sequence)
            })
            .find(|event| {
                if self.can_revert(event) {
                    true
                } else {
                    blocked_sequences.push(event.sequence);
                    false
                }
            })
            .cloned();

        let Some(target_event) = target_event else {
            if blocked_sequences.is_empty() {
                return Err(TimelineError::NothingToUndo(metadata.actor));
            }
            return Err(TimelineError::UndoConflict {
                actor: metadata.actor,
                blocked_sequences,
            });
        };

        let outcome = self.apply_internal(
            CommandEnvelope::new(metadata, target_event.inverse.clone()),
            Some(target_event.sequence),
        )?;
        let ApplyOutcome::Applied(event) = outcome else {
            return Err(TimelineError::UndoBecameNoOp(target_event.sequence));
        };

        self.reverted_sequences.insert(target_event.sequence);
        Ok(event)
    }

    #[must_use]
    pub fn blame_cell(&self, point: GridPoint) -> Option<BlameEntry> {
        self.blame_target(&LevelTarget::Cell(point))
    }

    #[must_use]
    pub fn blame_entity(&self, entity_id: &EntityId) -> Option<BlameEntry> {
        self.blame_target(&LevelTarget::Entity(entity_id.clone()))
    }

    #[must_use]
    pub fn blame_decorator(&self, decorator_id: &DecoratorId) -> Option<BlameEntry> {
        self.blame_target(&LevelTarget::Decorator(decorator_id.clone()))
    }

    #[must_use]
    pub fn blame_blind_tile(
        &self,
        entity_id: &EntityId,
        cell: crate::ShapeCell,
    ) -> Option<BlameEntry> {
        self.blame_target(&LevelTarget::BlindTile {
            entity_id: entity_id.clone(),
            cell,
        })
    }

    #[must_use]
    pub fn blame_blind_guide(&self, entity_id: &EntityId, guide: BlindGuide) -> Option<BlameEntry> {
        self.blame_target(&LevelTarget::BlindGuide {
            entity_id: entity_id.clone(),
            guide,
        })
    }

    #[must_use]
    pub fn history_for_cell(&self, point: GridPoint) -> Vec<&HistoryEvent> {
        self.history_for_target(&LevelTarget::Cell(point))
    }

    #[must_use]
    pub fn history_for_entity(&self, entity_id: &EntityId) -> Vec<&HistoryEvent> {
        self.history_for_target(&LevelTarget::Entity(entity_id.clone()))
    }

    #[must_use]
    pub fn history_for_decorator(&self, decorator_id: &DecoratorId) -> Vec<&HistoryEvent> {
        self.history_for_target(&LevelTarget::Decorator(decorator_id.clone()))
    }

    fn apply_internal(
        &mut self,
        envelope: CommandEnvelope,
        reverts_sequence: Option<u64>,
    ) -> Result<ApplyOutcome, TimelineError> {
        if self.command_sequences.contains_key(&envelope.metadata.id) {
            return Err(TimelineError::DuplicateCommandId(envelope.metadata.id));
        }

        let before_hash = self.current.content_hash();
        let Some(application) = apply_command(&self.current, &envelope.command)? else {
            return Ok(ApplyOutcome::NoChange {
                snapshot_hash: before_hash,
            });
        };

        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(TimelineError::SequenceExhausted)?;
        let event = HistoryEvent {
            sequence,
            metadata: envelope.metadata,
            command: envelope.command,
            inverse: application.inverse,
            changes: application.changes,
            reverts_sequence,
            before_hash,
            after_hash: application.snapshot.content_hash(),
        };

        self.current = application.snapshot;
        for change in &event.changes {
            self.provenance.insert(change.target.clone(), sequence);
        }
        self.command_sequences
            .insert(event.metadata.id.clone(), sequence);
        self.events.push(event.clone());
        Ok(ApplyOutcome::Applied(event))
    }

    fn can_revert(&self, event: &HistoryEvent) -> bool {
        let targets_available = event.changes.iter().all(|change| {
            self.current_value(&change.target) == Ok(change.after.clone())
                && self
                    .events
                    .iter()
                    .skip(event.sequence as usize)
                    .all(|later| {
                        let touches_target = later
                            .changes
                            .iter()
                            .any(|later_change| later_change.target == change.target);
                        !touches_target
                            || later.reverts_sequence.is_some()
                            || self.reverted_sequences.contains(&later.sequence)
                    })
        });
        if !targets_available {
            return false;
        }

        let Ok(Some(inverse)) = apply_command(&self.current, &event.inverse) else {
            return false;
        };
        inverse.changes.len() == event.changes.len()
            && event.changes.iter().all(|original| {
                inverse.changes.iter().any(|change| {
                    change.target == original.target
                        && change.before == original.after
                        && change.after == original.before
                })
            })
    }

    fn current_value(&self, target: &LevelTarget) -> Result<LevelValue, TimelineError> {
        match target {
            LevelTarget::Grid => Ok(LevelValue::Grid(Box::new(self.current.clone()))),
            LevelTarget::Cell(point) => Ok(LevelValue::Cell(self.current.cell(*point)?)),
            LevelTarget::Entity(entity_id) => {
                Ok(LevelValue::Entity(self.current.entity(entity_id).cloned()))
            }
            LevelTarget::Decorator(decorator_id) => Ok(LevelValue::Decorator(
                self.current.decorator(decorator_id).cloned(),
            )),
            LevelTarget::BlindTile { entity_id, cell } => {
                let tile = self
                    .current
                    .entity(entity_id)
                    .and_then(PlaceableEntity::as_blind)
                    .and_then(|blind| {
                        let shape = self.current.entity(entity_id)?.shape();
                        blind.tile_for_cell(shape, *cell)
                    })
                    .cloned();
                Ok(LevelValue::BlindTile(tile))
            }
            LevelTarget::BlindGuide { entity_id, guide } => {
                let present = self
                    .current
                    .entity(entity_id)
                    .and_then(PlaceableEntity::as_blind)
                    .is_some_and(|blind| blind.has_guide(*guide));
                Ok(LevelValue::BlindGuide(present))
            }
        }
    }

    fn blame_target(&self, target: &LevelTarget) -> Option<BlameEntry> {
        let sequence = self.provenance.get(target)?;
        let event = self.event(*sequence)?;
        Some(BlameEntry {
            sequence: event.sequence,
            command_id: event.metadata.id.clone(),
            actor: event.metadata.actor.clone(),
            occurred_at_ms: event.metadata.occurred_at_ms,
            command_kind: event.command.kind(),
            reverts_sequence: event.reverts_sequence,
        })
    }

    fn history_for_target(&self, target: &LevelTarget) -> Vec<&HistoryEvent> {
        self.events
            .iter()
            .filter(|event| event.changes.iter().any(|change| &change.target == target))
            .collect()
    }

    fn event(&self, sequence: u64) -> Option<&HistoryEvent> {
        let index = usize::try_from(sequence.checked_sub(1)?).ok()?;
        self.events.get(index)
    }
}

struct CommandApplication {
    snapshot: LevelSnapshot,
    inverse: LevelCommand,
    changes: Vec<HistoryChange>,
}

fn apply_command(
    snapshot: &LevelSnapshot,
    command: &LevelCommand,
) -> Result<Option<CommandApplication>, TimelineError> {
    match command {
        LevelCommand::ResizeGrid { size, anchor } => apply_resize_grid(snapshot, *size, *anchor),
        LevelCommand::RestoreSnapshot { snapshot: restored } => {
            apply_restore_snapshot(snapshot, restored)
        }
        LevelCommand::SetCell { point, kind } => apply_set_cell(snapshot, *point, *kind),
        LevelCommand::PlaceEntity { entity } => apply_place_entity(snapshot, entity),
        LevelCommand::PlaceEntities { entities } => apply_place_entities(snapshot, entities),
        LevelCommand::MoveEntity { entity_id, origin } => {
            let before = required_entity(snapshot, entity_id)?;
            if before.origin() == *origin {
                return Ok(None);
            }
            apply_entity_replacement(
                snapshot,
                before.moved_to(*origin),
                LevelCommand::MoveEntity {
                    entity_id: entity_id.clone(),
                    origin: before.origin(),
                },
            )
        }
        LevelCommand::MoveEntities { moves } => apply_move_entities(snapshot, moves),
        LevelCommand::TransformEntities { entities } => {
            apply_transform_entities(snapshot, entities)
        }
        LevelCommand::RotateEntityClockwise { entity_id } => {
            let before = required_entity(snapshot, entity_id)?;
            let after = before.rotated_clockwise();
            if &after == before {
                return Ok(None);
            }
            apply_entity_replacement(
                snapshot,
                after,
                LevelCommand::RestoreEntity {
                    entity: before.clone(),
                },
            )
        }
        LevelCommand::FlipEntityHorizontal { entity_id } => {
            let before = required_entity(snapshot, entity_id)?;
            let after = before.flipped_horizontal();
            if &after == before {
                return Ok(None);
            }
            apply_entity_replacement(
                snapshot,
                after,
                LevelCommand::FlipEntityHorizontal {
                    entity_id: entity_id.clone(),
                },
            )
        }
        LevelCommand::SetBlindResolution {
            entity_id,
            pixels_per_cell,
        } => {
            let before = required_entity(snapshot, entity_id)?;
            let after = before
                .resampled_blind(*pixels_per_cell)
                .map_err(LevelError::from)?;
            if &after == before {
                return Ok(None);
            }
            apply_entity_replacement(
                snapshot,
                after,
                LevelCommand::RestoreEntity {
                    entity: before.clone(),
                },
            )
        }
        LevelCommand::SetBlockCollectLayers { entity_id, layers } => {
            apply_set_block_collect_layers(snapshot, entity_id, layers)
        }
        LevelCommand::SetBlockLayerCapacity {
            entity_ids,
            layer_index,
            capacity,
        } => apply_set_block_layer_capacity(snapshot, entity_ids, *layer_index, *capacity),
        LevelCommand::DeleteEntity { entity_id } => apply_delete_entity(snapshot, entity_id),
        LevelCommand::DeleteEntities { entity_ids } => apply_delete_entities(snapshot, entity_ids),
        LevelCommand::RestoreEntity { entity } => {
            let before = required_entity(snapshot, entity.id())?;
            if before == entity {
                return Ok(None);
            }
            apply_entity_replacement(
                snapshot,
                entity.clone(),
                LevelCommand::RestoreEntity {
                    entity: before.clone(),
                },
            )
        }
        LevelCommand::RestoreEntities { entities } => apply_restore_entities(snapshot, entities),
        LevelCommand::RestoreDeletedEntity { entity, decorators } => {
            apply_restore_deleted_entity(snapshot, entity, decorators)
        }
        LevelCommand::RestoreDeletedEntities {
            entities,
            decorators,
        } => apply_restore_deleted_entities(snapshot, entities, decorators),
        LevelCommand::ToggleIce {
            decorator_id,
            entity_id,
        } => {
            let before = validate_ordinary_decorator_identity(
                snapshot,
                decorator_id,
                entity_id,
                OrdinaryDecoratorKind::Ice,
            )?;
            let blocking_count = match before.as_ref().and_then(Decorator::ice_blocking_count) {
                None | Some(0) => 1,
                Some(_) => 0,
            };
            apply_decorator_value(
                snapshot,
                decorator_id,
                Some(Decorator::ice_with_blocking_count(
                    decorator_id.clone(),
                    entity_id.clone(),
                    blocking_count,
                )),
            )
        }
        LevelCommand::SetIce {
            decorator_id,
            entity_id,
            blocking_count,
        } => {
            validate_ordinary_decorator_identity(
                snapshot,
                decorator_id,
                entity_id,
                OrdinaryDecoratorKind::Ice,
            )?;
            apply_decorator_value(
                snapshot,
                decorator_id,
                Some(Decorator::ice_with_blocking_count(
                    decorator_id.clone(),
                    entity_id.clone(),
                    *blocking_count,
                )),
            )
        }
        LevelCommand::ToggleGlass {
            decorator_id,
            entity_id,
        } => {
            let before = validate_ordinary_decorator_identity(
                snapshot,
                decorator_id,
                entity_id,
                OrdinaryDecoratorKind::Glass,
            )?;
            let blocking_count = match before.as_ref().and_then(Decorator::glass_blocking_count) {
                None | Some(0) => 1,
                Some(_) => 0,
            };
            apply_decorator_value(
                snapshot,
                decorator_id,
                Some(Decorator::glass_with_blocking_count(
                    decorator_id.clone(),
                    entity_id.clone(),
                    blocking_count,
                )),
            )
        }
        LevelCommand::SetGlass {
            decorator_id,
            entity_id,
            blocking_count,
        } => {
            validate_ordinary_decorator_identity(
                snapshot,
                decorator_id,
                entity_id,
                OrdinaryDecoratorKind::Glass,
            )?;
            apply_decorator_value(
                snapshot,
                decorator_id,
                Some(Decorator::glass_with_blocking_count(
                    decorator_id.clone(),
                    entity_id.clone(),
                    *blocking_count,
                )),
            )
        }
        LevelCommand::CycleDirection {
            decorator_id,
            entity_id,
        } => {
            let before = validate_ordinary_decorator_identity(
                snapshot,
                decorator_id,
                entity_id,
                OrdinaryDecoratorKind::Direction,
            )?;
            let mode = match before
                .as_ref()
                .and_then(Decorator::direction_constraint)
                .unwrap_or_default()
            {
                DirectionMode::Disabled => DirectionMode::Horizontal,
                DirectionMode::Horizontal => DirectionMode::Vertical,
                DirectionMode::Vertical => DirectionMode::Disabled,
            };
            apply_direction(snapshot, decorator_id, entity_id, mode)
        }
        LevelCommand::SetDirection {
            decorator_id,
            entity_id,
            mode,
        } => {
            validate_ordinary_decorator_identity(
                snapshot,
                decorator_id,
                entity_id,
                OrdinaryDecoratorKind::Direction,
            )?;
            apply_direction(snapshot, decorator_id, entity_id, *mode)
        }
        LevelCommand::AssignKeyLocker {
            decorator_id,
            key_entity_id,
            lock_entity_id,
        } => {
            if let Some(existing) = snapshot.decorator(decorator_id) {
                if !matches!(existing.kind(), DecoratorKind::KeyLocker { .. }) {
                    return Err(LevelError::DecoratorIdentityConflict {
                        decorator_id: decorator_id.clone(),
                    }
                    .into());
                }
            }
            apply_decorator_value(
                snapshot,
                decorator_id,
                Some(Decorator::key_locker(
                    decorator_id.clone(),
                    lock_entity_id.clone(),
                    key_entity_id.clone(),
                )),
            )
        }
        LevelCommand::RemoveKeyLocker { decorator_id } => {
            if let Some(existing) = snapshot.decorator(decorator_id) {
                if !matches!(existing.kind(), DecoratorKind::KeyLocker { .. }) {
                    return Err(LevelError::DecoratorIdentityConflict {
                        decorator_id: decorator_id.clone(),
                    }
                    .into());
                }
            }
            apply_decorator_value(snapshot, decorator_id, None)
        }
        LevelCommand::RestoreDecorator {
            decorator_id,
            decorator,
        } => apply_decorator_value(snapshot, decorator_id, decorator.clone()),
        LevelCommand::PaintBlindStroke {
            entity_id,
            color_index,
            stroke,
        } => apply_blind_operation(
            snapshot,
            entity_id,
            &BlindBrushOperation::PaintStroke {
                color_index: *color_index,
                stroke: stroke.clone(),
            },
        ),
        LevelCommand::EraseBlindStroke { entity_id, stroke } => apply_blind_operation(
            snapshot,
            entity_id,
            &BlindBrushOperation::EraseStroke {
                stroke: stroke.clone(),
            },
        ),
        LevelCommand::FloodFillBlind {
            entity_id,
            start,
            color_index,
        } => apply_blind_operation(
            snapshot,
            entity_id,
            &BlindBrushOperation::FloodFill {
                start: *start,
                color_index: *color_index,
            },
        ),
        LevelCommand::AddBlindGuides { entity_id, guides } => apply_blind_operation(
            snapshot,
            entity_id,
            &BlindBrushOperation::AddGuides {
                guides: guides.clone(),
            },
        ),
        LevelCommand::RemoveBlindGuides { entity_id, guides } => apply_blind_operation(
            snapshot,
            entity_id,
            &BlindBrushOperation::RemoveGuides {
                guides: guides.clone(),
            },
        ),
        LevelCommand::RestoreBlindPatch {
            entity_id,
            tile_patches,
            guide_patches,
        } => apply_blind_restore(snapshot, entity_id, tile_patches, guide_patches),
    }
}

fn apply_resize_grid(
    snapshot: &LevelSnapshot,
    size: GridSize,
    anchor: GridAnchor,
) -> Result<Option<CommandApplication>, TimelineError> {
    size.validate()?;
    if snapshot.size() == size {
        return Ok(None);
    }

    let (offset_x, offset_y) = resize_offset(snapshot.size(), size, anchor);
    let mut cells = vec![CellKind::Floor; size.cell_count()];
    for y in 0..snapshot.size().height() {
        for x in 0..snapshot.size().width() {
            let source = GridPoint::new(x, y);
            let kind = snapshot.cell(source)?;
            let Some(target) = translated_point(source, offset_x, offset_y, size) else {
                if kind != CellKind::Floor {
                    return Err(LevelError::ResizeWouldClipCell { point: source }.into());
                }
                continue;
            };
            cells[size.index_of(target)?] = kind;
        }
    }

    let entities = snapshot
        .entities()
        .iter()
        .map(|entity| {
            translated_point(entity.origin(), offset_x, offset_y, size)
                .map(|origin| entity.moved_to(origin))
                .ok_or_else(|| LevelError::ResizeWouldClipEntity {
                    entity_id: entity.id().clone(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let resized = LevelSnapshot::from_parts(size, cells, entities, snapshot.decorators().to_vec())?;
    Ok(Some(snapshot_replacement(snapshot, resized)))
}

fn apply_restore_snapshot(
    snapshot: &LevelSnapshot,
    restored: &LevelSnapshot,
) -> Result<Option<CommandApplication>, TimelineError> {
    restored.validate_domain()?;
    if snapshot == restored {
        return Ok(None);
    }
    Ok(Some(snapshot_replacement(snapshot, restored.clone())))
}

fn snapshot_replacement(before: &LevelSnapshot, after: LevelSnapshot) -> CommandApplication {
    CommandApplication {
        inverse: LevelCommand::RestoreSnapshot {
            snapshot: Box::new(before.clone()),
        },
        changes: vec![HistoryChange {
            target: LevelTarget::Grid,
            before: LevelValue::Grid(Box::new(before.clone())),
            after: LevelValue::Grid(Box::new(after.clone())),
        }],
        snapshot: after,
    }
}

fn resize_offset(before: GridSize, after: GridSize, anchor: GridAnchor) -> (i32, i32) {
    let width_delta = i32::from(after.width()) - i32::from(before.width());
    let height_delta = i32::from(after.height()) - i32::from(before.height());
    let offset_x = match anchor {
        GridAnchor::TopLeft | GridAnchor::Left | GridAnchor::BottomLeft => 0,
        GridAnchor::Top | GridAnchor::Center | GridAnchor::Bottom => width_delta.div_euclid(2),
        GridAnchor::TopRight | GridAnchor::Right | GridAnchor::BottomRight => width_delta,
    };
    let offset_y = match anchor {
        GridAnchor::BottomLeft | GridAnchor::Bottom | GridAnchor::BottomRight => 0,
        GridAnchor::Left | GridAnchor::Center | GridAnchor::Right => height_delta.div_euclid(2),
        GridAnchor::TopLeft | GridAnchor::Top | GridAnchor::TopRight => height_delta,
    };
    (offset_x, offset_y)
}

fn translated_point(
    point: GridPoint,
    offset_x: i32,
    offset_y: i32,
    size: GridSize,
) -> Option<GridPoint> {
    let x = i32::from(point.x).checked_add(offset_x)?;
    let y = i32::from(point.y).checked_add(offset_y)?;
    if x < 0 || y < 0 || x >= i32::from(size.width()) || y >= i32::from(size.height()) {
        return None;
    }
    Some(GridPoint::new(x as u16, y as u16))
}

fn apply_set_cell(
    snapshot: &LevelSnapshot,
    point: GridPoint,
    kind: CellKind,
) -> Result<Option<CommandApplication>, TimelineError> {
    let before = snapshot.cell(point)?;
    if before == kind {
        return Ok(None);
    }
    if kind == CellKind::Wall {
        if let Some(entity) = snapshot.entity_at(point) {
            return Err(LevelError::CellOccupiedByEntity {
                point,
                entity_id: entity.id().clone(),
            }
            .into());
        }
    }

    let mut next = snapshot.clone();
    next.set_cell(point, kind)?;
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::SetCell {
            point,
            kind: before,
        },
        changes: vec![HistoryChange {
            target: LevelTarget::Cell(point),
            before: LevelValue::Cell(before),
            after: LevelValue::Cell(kind),
        }],
    }))
}

fn apply_place_entity(
    snapshot: &LevelSnapshot,
    entity: &PlaceableEntity,
) -> Result<Option<CommandApplication>, TimelineError> {
    let mut next = snapshot.clone();
    next.insert_entity(entity.clone())?;
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::DeleteEntity {
            entity_id: entity.id().clone(),
        },
        changes: vec![entity_change(
            entity.id().clone(),
            None,
            Some(entity.clone()),
        )],
    }))
}

fn apply_place_entities(
    snapshot: &LevelSnapshot,
    entities: &[PlaceableEntity],
) -> Result<Option<CommandApplication>, TimelineError> {
    if entities.is_empty() {
        return Ok(None);
    }
    let mut next = snapshot.clone();
    let mut ids = BTreeSet::new();
    let mut changes = Vec::with_capacity(entities.len());
    for entity in entities {
        if !ids.insert(entity.id().clone()) {
            return Err(LevelError::DuplicateEntityId(entity.id().clone()).into());
        }
        next.insert_entity(entity.clone())?;
        changes.push(entity_change(
            entity.id().clone(),
            None,
            Some(entity.clone()),
        ));
    }
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::DeleteEntities {
            entity_ids: ids.into_iter().collect(),
        },
        changes,
    }))
}

fn apply_transform_entities(
    snapshot: &LevelSnapshot,
    entities: &[PlaceableEntity],
) -> Result<Option<CommandApplication>, TimelineError> {
    if entities.is_empty() {
        return Ok(None);
    }
    let mut seen = BTreeSet::new();
    let mut replacements = Vec::with_capacity(entities.len());
    let mut before_entities = Vec::with_capacity(entities.len());
    let mut changes = Vec::with_capacity(entities.len());
    for after in entities {
        if !seen.insert(after.id().clone()) {
            return Err(LevelError::DuplicateEntityMove(after.id().clone()).into());
        }
        let before = required_entity(snapshot, after.id())?;
        if before == after {
            continue;
        }
        replacements.push(after.clone());
        before_entities.push(before.clone());
        changes.push(entity_change(
            after.id().clone(),
            Some(before.clone()),
            Some(after.clone()),
        ));
    }
    if replacements.is_empty() {
        return Ok(None);
    }
    let mut next = snapshot.clone();
    next.replace_entities(replacements)?;
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::RestoreEntities {
            entities: before_entities,
        },
        changes,
    }))
}

fn apply_restore_entities(
    snapshot: &LevelSnapshot,
    entities: &[PlaceableEntity],
) -> Result<Option<CommandApplication>, TimelineError> {
    if entities.is_empty() {
        return Ok(None);
    }
    let mut seen = BTreeSet::new();
    let mut replacements = Vec::with_capacity(entities.len());
    let mut before_entities = Vec::with_capacity(entities.len());
    let mut changes = Vec::with_capacity(entities.len());
    for after in entities {
        if !seen.insert(after.id().clone()) {
            return Err(LevelError::DuplicateEntityMove(after.id().clone()).into());
        }
        let before = required_entity(snapshot, after.id())?;
        if before == after {
            continue;
        }
        replacements.push(after.clone());
        before_entities.push(before.clone());
        changes.push(entity_change(
            after.id().clone(),
            Some(before.clone()),
            Some(after.clone()),
        ));
    }
    if replacements.is_empty() {
        return Ok(None);
    }
    let mut next = snapshot.clone();
    next.replace_entities(replacements)?;
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::TransformEntities {
            entities: before_entities,
        },
        changes,
    }))
}

fn apply_move_entities(
    snapshot: &LevelSnapshot,
    moves: &[EntityMove],
) -> Result<Option<CommandApplication>, TimelineError> {
    let mut seen = BTreeSet::new();
    let mut replacements = Vec::with_capacity(moves.len());
    let mut inverse_moves = Vec::with_capacity(moves.len());
    let mut changes = Vec::with_capacity(moves.len());
    for movement in moves {
        if !seen.insert(movement.entity_id.clone()) {
            return Err(LevelError::DuplicateEntityMove(movement.entity_id.clone()).into());
        }
        let before = required_entity(snapshot, &movement.entity_id)?;
        if before.origin() == movement.origin {
            continue;
        }
        let after = before.moved_to(movement.origin);
        replacements.push(after.clone());
        inverse_moves.push(EntityMove::new(before.id().clone(), before.origin()));
        changes.push(entity_change(
            before.id().clone(),
            Some(before.clone()),
            Some(after),
        ));
    }
    if replacements.is_empty() {
        return Ok(None);
    }

    let mut next = snapshot.clone();
    next.replace_entities(replacements)?;
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::MoveEntities {
            moves: inverse_moves,
        },
        changes,
    }))
}

fn apply_entity_replacement(
    snapshot: &LevelSnapshot,
    after: PlaceableEntity,
    inverse: LevelCommand,
) -> Result<Option<CommandApplication>, TimelineError> {
    let mut next = snapshot.clone();
    let before = next.replace_entity(after.clone())?;
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse,
        changes: vec![entity_change(after.id().clone(), Some(before), Some(after))],
    }))
}

fn apply_set_block_collect_layers(
    snapshot: &LevelSnapshot,
    entity_id: &EntityId,
    layers: &[CollectLayer],
) -> Result<Option<CommandApplication>, TimelineError> {
    let before = required_entity(snapshot, entity_id)?;
    let after = before
        .with_block_collect_layers(layers.to_vec())
        .map_err(LevelError::from)?;
    apply_transform_entities(snapshot, &[after])
}

fn apply_set_block_layer_capacity(
    snapshot: &LevelSnapshot,
    entity_ids: &[EntityId],
    layer_index: usize,
    capacity: CollectCapacity,
) -> Result<Option<CommandApplication>, TimelineError> {
    if entity_ids.is_empty() {
        return Ok(None);
    }
    let mut seen = BTreeSet::new();
    let mut entities = Vec::with_capacity(entity_ids.len());
    for entity_id in entity_ids {
        if !seen.insert(entity_id.clone()) {
            return Err(LevelError::DuplicateEntityMove(entity_id.clone()).into());
        }
        let before = required_entity(snapshot, entity_id)?;
        let layers = match before.kind() {
            crate::PlaceableEntityKind::Block(block) => block.collect_layers(),
            crate::PlaceableEntityKind::Blind(_) => {
                return Err(LevelError::from(EntityError::NotBlock(entity_id.clone())).into());
            }
        };
        let Some(layer) = layers.get(layer_index) else {
            return Err(LevelError::from(EntityError::CollectLayerNotFound {
                entity_id: entity_id.clone(),
                layer_index,
            })
            .into());
        };
        let mut next_layers = layers.to_vec();
        next_layers[layer_index] = CollectLayer::new(
            layer.color_index(),
            layer.radius(),
            capacity,
            layer.is_locked(),
        );
        entities.push(
            before
                .with_block_collect_layers(next_layers)
                .map_err(LevelError::from)?,
        );
    }
    apply_transform_entities(snapshot, &entities)
}

fn apply_delete_entity(
    snapshot: &LevelSnapshot,
    entity_id: &EntityId,
) -> Result<Option<CommandApplication>, TimelineError> {
    let mut next = snapshot.clone();
    let (entity, decorators) = next.remove_entity_cascade(entity_id)?;
    let mut changes = vec![entity_change(
        entity.id().clone(),
        Some(entity.clone()),
        None,
    )];
    changes.extend(
        decorators
            .iter()
            .cloned()
            .map(|decorator| decorator_change(decorator.id().clone(), Some(decorator), None)),
    );
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::RestoreDeletedEntity { entity, decorators },
        changes,
    }))
}

fn apply_delete_entities(
    snapshot: &LevelSnapshot,
    entity_ids: &[EntityId],
) -> Result<Option<CommandApplication>, TimelineError> {
    if entity_ids.is_empty() {
        return Ok(None);
    }
    let mut next = snapshot.clone();
    let mut seen = BTreeSet::new();
    let mut entities = Vec::with_capacity(entity_ids.len());
    let mut decorators = BTreeMap::new();
    let mut changes = Vec::new();
    for entity_id in entity_ids {
        if !seen.insert(entity_id.clone()) {
            return Err(LevelError::DuplicateEntityMove(entity_id.clone()).into());
        }
        let (entity, removed_decorators) = next.remove_entity_cascade(entity_id)?;
        changes.push(entity_change(
            entity.id().clone(),
            Some(entity.clone()),
            None,
        ));
        entities.push(entity);
        for decorator in removed_decorators {
            decorators
                .entry(decorator.id().clone())
                .or_insert_with(|| decorator.clone());
        }
    }
    let decorators: Vec<_> = decorators.into_values().collect();
    changes.extend(
        decorators
            .iter()
            .cloned()
            .map(|decorator| decorator_change(decorator.id().clone(), Some(decorator), None)),
    );
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::RestoreDeletedEntities {
            entities,
            decorators,
        },
        changes,
    }))
}

fn apply_restore_deleted_entity(
    snapshot: &LevelSnapshot,
    entity: &PlaceableEntity,
    decorators: &[Decorator],
) -> Result<Option<CommandApplication>, TimelineError> {
    let mut next = snapshot.clone();
    next.restore_entity_cascade(entity.clone(), decorators)?;
    let mut changes = vec![entity_change(
        entity.id().clone(),
        None,
        Some(entity.clone()),
    )];
    changes.extend(
        decorators
            .iter()
            .cloned()
            .map(|decorator| decorator_change(decorator.id().clone(), None, Some(decorator))),
    );
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::DeleteEntity {
            entity_id: entity.id().clone(),
        },
        changes,
    }))
}

fn apply_restore_deleted_entities(
    snapshot: &LevelSnapshot,
    entities: &[PlaceableEntity],
    decorators: &[Decorator],
) -> Result<Option<CommandApplication>, TimelineError> {
    if entities.is_empty() {
        return Ok(None);
    }
    let mut next = snapshot.clone();
    for entity in entities {
        next.insert_entity(entity.clone())?;
    }
    for decorator in decorators {
        if next.decorator(decorator.id()).is_some() {
            return Err(LevelError::DuplicateDecoratorId(decorator.id().clone()).into());
        }
        next.put_decorator(decorator.clone())?;
    }
    let mut changes = entities
        .iter()
        .cloned()
        .map(|entity| entity_change(entity.id().clone(), None, Some(entity)))
        .collect::<Vec<_>>();
    changes.extend(
        decorators
            .iter()
            .cloned()
            .map(|decorator| decorator_change(decorator.id().clone(), None, Some(decorator))),
    );
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::DeleteEntities {
            entity_ids: entities.iter().map(|entity| entity.id().clone()).collect(),
        },
        changes,
    }))
}

fn apply_direction(
    snapshot: &LevelSnapshot,
    decorator_id: &DecoratorId,
    entity_id: &EntityId,
    mode: DirectionMode,
) -> Result<Option<CommandApplication>, TimelineError> {
    apply_decorator_value(
        snapshot,
        decorator_id,
        Decorator::direction_mode(decorator_id.clone(), entity_id.clone(), mode),
    )
}

fn apply_decorator_value(
    snapshot: &LevelSnapshot,
    decorator_id: &DecoratorId,
    after: Option<Decorator>,
) -> Result<Option<CommandApplication>, TimelineError> {
    if after
        .as_ref()
        .is_some_and(|decorator| decorator.id() != decorator_id)
    {
        return Err(LevelError::DecoratorIdentityConflict {
            decorator_id: decorator_id.clone(),
        }
        .into());
    }
    let before = snapshot.decorator(decorator_id).cloned();
    if before == after {
        return Ok(None);
    }

    let mut next = snapshot.clone();
    match after.clone() {
        Some(decorator) => {
            next.put_decorator(decorator)?;
        }
        None => {
            next.remove_decorator(decorator_id);
        }
    }
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::RestoreDecorator {
            decorator_id: decorator_id.clone(),
            decorator: before.clone(),
        },
        changes: vec![HistoryChange {
            target: LevelTarget::Decorator(decorator_id.clone()),
            before: LevelValue::Decorator(before),
            after: LevelValue::Decorator(after),
        }],
    }))
}

fn apply_blind_operation(
    snapshot: &LevelSnapshot,
    entity_id: &EntityId,
    operation: &BlindBrushOperation,
) -> Result<Option<CommandApplication>, TimelineError> {
    let before = required_entity(snapshot, entity_id)?;
    let after = before
        .apply_blind_operation(operation)
        .map_err(LevelError::from)?;
    apply_blind_replacement(snapshot, before, after)
}

fn apply_blind_restore(
    snapshot: &LevelSnapshot,
    entity_id: &EntityId,
    tile_patches: &[BlindTilePatch],
    guide_patches: &[BlindGuidePatch],
) -> Result<Option<CommandApplication>, TimelineError> {
    let before = required_entity(snapshot, entity_id)?;
    let after = before
        .restore_blind_patch(tile_patches, guide_patches)
        .map_err(LevelError::from)?;
    apply_blind_replacement(snapshot, before, after)
}

fn apply_blind_replacement(
    snapshot: &LevelSnapshot,
    before: &PlaceableEntity,
    after: PlaceableEntity,
) -> Result<Option<CommandApplication>, TimelineError> {
    if before == &after {
        return Ok(None);
    }
    let changes = blind_changes(before, &after)?;
    let (tile_patches, guide_patches) = inverse_blind_patches(&changes);
    let mut next = snapshot.clone();
    next.replace_entity(after)?;
    Ok(Some(CommandApplication {
        snapshot: next,
        inverse: LevelCommand::RestoreBlindPatch {
            entity_id: before.id().clone(),
            tile_patches,
            guide_patches,
        },
        changes,
    }))
}

fn blind_changes(
    before: &PlaceableEntity,
    after: &PlaceableEntity,
) -> Result<Vec<HistoryChange>, TimelineError> {
    let before_blind = before
        .as_blind()
        .ok_or_else(|| EntityError::NotBlind(before.id().clone()))
        .map_err(LevelError::from)?;
    let after_blind = after
        .as_blind()
        .ok_or_else(|| EntityError::NotBlind(after.id().clone()))
        .map_err(LevelError::from)?;
    let mut changes = Vec::new();
    for ((cell, before_tile), after_tile) in before
        .shape()
        .occupied_cells()
        .zip(before_blind.tiles())
        .zip(after_blind.tiles())
    {
        if before_tile != after_tile {
            changes.push(HistoryChange {
                target: LevelTarget::BlindTile {
                    entity_id: before.id().clone(),
                    cell,
                },
                before: LevelValue::BlindTile(Some(before_tile.clone())),
                after: LevelValue::BlindTile(Some(after_tile.clone())),
            });
        }
    }

    let guides: BTreeSet<_> = before_blind
        .guides()
        .iter()
        .chain(after_blind.guides())
        .copied()
        .collect();
    for guide in guides {
        let was_present = before_blind.has_guide(guide);
        let is_present = after_blind.has_guide(guide);
        if was_present != is_present {
            changes.push(HistoryChange {
                target: LevelTarget::BlindGuide {
                    entity_id: before.id().clone(),
                    guide,
                },
                before: LevelValue::BlindGuide(was_present),
                after: LevelValue::BlindGuide(is_present),
            });
        }
    }
    Ok(changes)
}

fn inverse_blind_patches(changes: &[HistoryChange]) -> (Vec<BlindTilePatch>, Vec<BlindGuidePatch>) {
    let mut tile_patches = Vec::new();
    let mut guide_patches = Vec::new();
    for change in changes {
        match (&change.target, &change.before) {
            (LevelTarget::BlindTile { cell, .. }, LevelValue::BlindTile(Some(tile))) => {
                tile_patches.push(BlindTilePatch {
                    cell: *cell,
                    tile: tile.clone(),
                })
            }
            (LevelTarget::BlindGuide { guide, .. }, LevelValue::BlindGuide(present)) => {
                guide_patches.push(BlindGuidePatch {
                    guide: *guide,
                    present: *present,
                });
            }
            _ => {}
        }
    }
    (tile_patches, guide_patches)
}

fn required_entity<'a>(
    snapshot: &'a LevelSnapshot,
    entity_id: &EntityId,
) -> Result<&'a PlaceableEntity, TimelineError> {
    snapshot
        .entity(entity_id)
        .ok_or_else(|| LevelError::EntityNotFound(entity_id.clone()).into())
}

#[derive(Clone, Copy)]
enum OrdinaryDecoratorKind {
    Ice,
    Glass,
    Direction,
}

fn validate_ordinary_decorator_identity(
    snapshot: &LevelSnapshot,
    decorator_id: &DecoratorId,
    entity_id: &EntityId,
    expected: OrdinaryDecoratorKind,
) -> Result<Option<Decorator>, TimelineError> {
    if snapshot.entity(entity_id).is_none() {
        return Err(LevelError::EntityNotFound(entity_id.clone()).into());
    }
    let existing = snapshot.decorator(decorator_id).cloned();
    if let Some(decorator) = &existing {
        let valid = match (expected, decorator.kind()) {
            (OrdinaryDecoratorKind::Ice, DecoratorKind::Ice { entity, .. })
            | (OrdinaryDecoratorKind::Glass, DecoratorKind::Glass { entity, .. })
            | (OrdinaryDecoratorKind::Direction, DecoratorKind::Direction { entity, .. }) => {
                entity == entity_id
            }
            _ => false,
        };
        if !valid {
            return Err(LevelError::DecoratorIdentityConflict {
                decorator_id: decorator_id.clone(),
            }
            .into());
        }
    }

    let owner = match expected {
        OrdinaryDecoratorKind::Ice => snapshot.ice_for_entity(entity_id),
        OrdinaryDecoratorKind::Glass => snapshot.glass_for_entity(entity_id),
        OrdinaryDecoratorKind::Direction => snapshot.direction_for_entity(entity_id),
    };
    if owner.is_some_and(|decorator| decorator.id() != decorator_id) {
        return Err(LevelError::DecoratorIdentityConflict {
            decorator_id: decorator_id.clone(),
        }
        .into());
    }
    Ok(existing)
}

fn entity_change(
    entity_id: EntityId,
    before: Option<PlaceableEntity>,
    after: Option<PlaceableEntity>,
) -> HistoryChange {
    HistoryChange {
        target: LevelTarget::Entity(entity_id),
        before: LevelValue::Entity(before),
        after: LevelValue::Entity(after),
    }
}

fn decorator_change(
    decorator_id: DecoratorId,
    before: Option<Decorator>,
    after: Option<Decorator>,
) -> HistoryChange {
    HistoryChange {
        target: LevelTarget::Decorator(decorator_id),
        before: LevelValue::Decorator(before),
        after: LevelValue::Decorator(after),
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TimelineError {
    #[error(transparent)]
    InvalidGrid(#[from] GridError),

    #[error(transparent)]
    InvalidLevel(#[from] LevelError),

    #[error("command ID '{0}' was already accepted")]
    DuplicateCommandId(CommandId),

    #[error("actor '{0}' has no active command to undo")]
    NothingToUndo(ActorId),

    #[error(
        "actor '{actor}' cannot undo because later active changes block sequences {blocked_sequences:?}"
    )]
    UndoConflict {
        actor: ActorId,
        blocked_sequences: Vec<u64>,
    },

    #[error("undo of sequence {0} became a no-op")]
    UndoBecameNoOp(u64),

    #[error("the command sequence is exhausted")]
    SequenceExhausted,
}
