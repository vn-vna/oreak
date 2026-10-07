//! Lossless compatibility between Unity `LevelData` JSON and `oreak-core`.
//!
//! The adapter owns the legacy wire sidecar. `oreak-core` remains the semantic
//! authority for grids, shapes, placements, entities, and decorators.

mod codec;
mod json;

use std::collections::{HashMap, HashSet};

pub use codec::{CodecError, DATA_CODEC_SALT_SIZE, DataCodec, DecodedData};
use codec::{decode_grid, encode_with_metadata};
use json::JsonNode;
use oreak_core::{
    Blind, BlindTile, Block, CardinalDirection, CellKind, CollectCapacity, CollectLayer, Decorator,
    DecoratorId, DecoratorKind, EntityId, GridPoint, GridSize, LevelSnapshot, PlaceableEntity,
    PlaceableEntityKind, PoolBoundary, Shape,
};
use thiserror::Error;

pub const DEFAULT_COLLECT_RADIUS_PIXELS: u8 = 20;
pub const DEFAULT_BLIND_PIXELS_PER_CELL: u8 = 32;
pub const DISABLED_COLLECT_COLOR_INDEX: u16 = u16::MAX;

const ENTITIES_PROPERTY: &str = "entitites";
const SALT_SIZE: usize = DATA_CODEC_SALT_SIZE;
const MAX_COLLECT_ROWS: usize = u16::MAX as usize;
const SBLN_HEADER_SIZE: usize = 9;
const SBCL_V2_HEADER_SIZE: usize = 7;

/// A project codec acknowledges and validates one otherwise unknown marker.
///
/// Unknown envelopes remain byte-for-token preserved. The plugin is responsible
/// for rejecting payloads whose references cannot safely coexist with core edits.
pub trait UnknownEntityCodec {
    fn marker(&self) -> &str;
    fn validate(&self, payload: &serde_json::Value) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompatibilityState {
    Writable,
    ReadOnly { unknown_markers: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyEntityInfo {
    pub index: usize,
    pub id: String,
    pub marker: String,
    pub known: bool,
}

#[derive(Clone, Debug)]
pub struct LegacyLevel {
    source: JsonNode,
    snapshot: LevelSnapshot,
    duration: f32,
    records: Vec<LegacyRecord>,
    tombstones: Vec<TombstoneRecord>,
    tombstone_sidecar_writable: bool,
    compatibility: CompatibilityState,
}

#[derive(Clone, Debug)]
struct LegacyRecord {
    id: String,
    marker: String,
    envelope: JsonNode,
    payload: JsonNode,
    kind: RecordKind,
}

#[derive(Clone, Debug)]
enum TombstoneRecord {
    Glass(LegacyRecord),
    Opaque {
        id: Option<String>,
        envelope: JsonNode,
    },
}

#[derive(Clone, Debug)]
enum RecordKind {
    Block {
        metadata: GridMetadata,
    },
    Blind {
        metadata: GridMetadata,
        wire_pixels: Vec<u8>,
        placeholder: bool,
    },
    Ice {
        count: u32,
    },
    Glass {
        count: u32,
    },
    Direction,
    KeyLocker,
    Unknown,
}

#[derive(Clone, Debug)]
enum GridMetadata {
    None,
    Sbln { width: u16, height: u16 },
    SbclV1,
    SbclV2 { locks: Vec<bool> },
}

#[derive(Clone, Debug)]
struct PendingBlind {
    record_index: usize,
    id: String,
    origin: GridPoint,
    shape: Shape,
    canvas: CanvasDescriptor,
    boundary: PoolBoundary,
}

#[derive(Clone, Debug)]
struct CanvasDescriptor {
    width: usize,
    height: usize,
    encoded: Option<String>,
    placeholder: bool,
}

impl LegacyLevel {
    pub fn parse(json: &str) -> Result<Self, LegacyError> {
        Self::parse_with_codecs(json, &[])
    }

    pub fn parse_with_codecs(
        json: &str,
        codecs: &[&dyn UnknownEntityCodec],
    ) -> Result<Self, LegacyError> {
        validate_codec_markers(codecs)?;
        let source = JsonNode::parse_root(json).map_err(|error| LegacyError::Json {
            offset: error.offset,
            message: error.message,
        })?;
        let duration = parse_duration(required(&source, "dur")?)?;
        let size = parse_board_size(required(&source, "bes")?)?;
        let cells = parse_board_cells(required(&source, "bdat")?, size)?;
        let envelopes = required(&source, ENTITIES_PROPERTY)?
            .as_array()
            .ok_or_else(|| invalid(ENTITIES_PROPERTY, "must be an array"))?;

        let mut records = Vec::with_capacity(envelopes.len());
        let mut placeables = Vec::new();
        let mut pending_blinds = Vec::new();
        let mut pending_decorators = Vec::new();
        let mut ids = HashSet::new();
        let mut unknown_without_codec = HashSet::new();
        let mut inferred_resolution = None;

        for (index, envelope) in envelopes.iter().enumerate() {
            let path = format!("{ENTITIES_PROPERTY}[{index}]");
            let (marker, payload) = parse_envelope(envelope, &path)?;
            let id = parse_nonempty_string(required(payload, "eid")?, &format!("{path}[1].eid"))?;
            if !ids.insert(id.clone()) {
                return Err(LegacyError::DuplicateEntityId(id));
            }

            let kind = match marker.as_str() {
                "block" => {
                    let (origin, shape, metadata) = parse_grid(payload, &path, false)?;
                    let layers = parse_collect_layers(payload.get("cc"), &metadata, &path)?;
                    let entity =
                        PlaceableEntity::block(id.clone(), origin, shape, Block::new(layers))
                            .map_err(domain)?;
                    placeables.push(entity);
                    RecordKind::Block { metadata }
                }
                "pool" => {
                    let (origin, shape, metadata) = parse_grid(payload, &path, true)?;
                    let canvas = parse_canvas(payload, shape, &path)?;
                    if !canvas.placeholder {
                        let resolution = canvas.width / usize::from(shape.width());
                        if let Some(existing) = inferred_resolution {
                            if existing != resolution {
                                return Err(invalid(
                                    format!("{path}[1].stc.c.r"),
                                    format!(
                                        "implies {resolution} pixels/cell, but the level already uses {existing}"
                                    ),
                                ));
                            }
                        } else {
                            inferred_resolution = Some(resolution);
                        }
                    }
                    pending_blinds.push(PendingBlind {
                        record_index: records.len(),
                        id: id.clone(),
                        origin,
                        shape,
                        canvas,
                        boundary: parse_pool_boundary(payload, &path)?,
                    });
                    RecordKind::Blind {
                        metadata,
                        wire_pixels: Vec::new(),
                        placeholder: false,
                    }
                }
                "ice" => {
                    let target = parse_decorator_target(payload, &path, "deco")?;
                    let count =
                        parse_u32(required(payload, "count")?, &format!("{path}[1].count"))?;
                    pending_decorators.push(PendingDecorator {
                        id: id.clone(),
                        kind: PendingDecoratorKind::Ice { target, count },
                    });
                    RecordKind::Ice { count }
                }
                "glass" => {
                    let target = parse_decorator_target(payload, &path, "deco")?;
                    let count =
                        parse_u32(required(payload, "count")?, &format!("{path}[1].count"))?;
                    if count == 0 {
                        return Err(invalid(
                            format!("{path}[1].count"),
                            "zero-count Glass must be stored in _led.dt",
                        ));
                    }
                    pending_decorators.push(PendingDecorator {
                        id: id.clone(),
                        kind: PendingDecoratorKind::Glass { target, count },
                    });
                    RecordKind::Glass { count }
                }
                "direction" => {
                    let target = parse_decorator_target(payload, &path, "deco")?;
                    let direction = match required(payload, "dir")?.as_str() {
                        Some("Horizontal") => CardinalDirection::Right,
                        Some("Vertical") => CardinalDirection::Up,
                        _ => {
                            return Err(invalid(
                                format!("{path}[1].dir"),
                                "must be exactly 'Horizontal' or 'Vertical'",
                            ));
                        }
                    };
                    pending_decorators.push(PendingDecorator {
                        id: id.clone(),
                        kind: PendingDecoratorKind::Direction { target, direction },
                    });
                    RecordKind::Direction
                }
                "key-locker" => {
                    let key = parse_decorator_target(payload, &path, "deco")?;
                    let locker = parse_decorator_target(payload, &path, "lock")?;
                    pending_decorators.push(PendingDecorator {
                        id: id.clone(),
                        kind: PendingDecoratorKind::KeyLocker { key, locker },
                    });
                    RecordKind::KeyLocker
                }
                _ => {
                    if let Some(codec) = codecs.iter().find(|codec| codec.marker() == marker) {
                        codec
                            .validate(&payload.to_serde_value())
                            .map_err(|message| LegacyError::Plugin {
                                marker: marker.clone(),
                                message,
                            })?;
                    } else {
                        unknown_without_codec.insert(marker.clone());
                    }
                    RecordKind::Unknown
                }
            };
            records.push(LegacyRecord {
                id,
                marker,
                envelope: envelope.clone(),
                payload: payload.clone(),
                kind,
            });
        }
        let (tombstones, tombstone_sidecar_writable) =
            parse_glass_tombstones(&source, &mut ids, &mut pending_decorators)?;

        let resolution = inferred_resolution.unwrap_or(usize::from(DEFAULT_BLIND_PIXELS_PER_CELL));
        for pending in pending_blinds {
            let (blind, wire_pixels) = decode_blind(&pending, resolution)?;
            if let RecordKind::Blind {
                metadata,
                wire_pixels: target,
                placeholder,
            } = &mut records[pending.record_index].kind
            {
                if let GridMetadata::Sbln { width, height } = metadata {
                    let expected_width = usize::from(*width);
                    let expected_height = usize::from(*height);
                    let actual_width = usize::from(pending.shape.width()) * resolution;
                    let actual_height = usize::from(pending.shape.height()) * resolution;
                    if (expected_width, expected_height) != (actual_width, actual_height) {
                        return Err(invalid(
                            format!("{} grid SBLN metadata", pending.id),
                            format!(
                                "dimensions {expected_width} x {expected_height} do not match canvas {actual_width} x {actual_height}"
                            ),
                        ));
                    }
                }
                *target = wire_pixels;
                *placeholder = pending.canvas.placeholder;
            }
            placeables.push(
                PlaceableEntity::blind(pending.id, pending.origin, pending.shape, blind)
                    .map_err(domain)?,
            );
        }

        let decorators = resolve_decorators(&placeables, pending_decorators)?;
        let snapshot =
            LevelSnapshot::from_parts(size, cells, placeables, decorators).map_err(domain)?;
        validate_legacy_semantics(&snapshot)?;

        let compatibility = if unknown_without_codec.is_empty() {
            CompatibilityState::Writable
        } else {
            let mut unknown_markers: Vec<_> = unknown_without_codec.into_iter().collect();
            unknown_markers.sort();
            CompatibilityState::ReadOnly { unknown_markers }
        };
        Ok(Self {
            source,
            snapshot,
            duration,
            records,
            tombstones,
            tombstone_sidecar_writable,
            compatibility,
        })
    }

    #[must_use]
    pub const fn snapshot(&self) -> &LevelSnapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn duration(&self) -> f32 {
        self.duration
    }

    #[must_use]
    pub const fn compatibility_state(&self) -> &CompatibilityState {
        &self.compatibility
    }

    #[must_use]
    pub fn entity_order(&self) -> Vec<LegacyEntityInfo> {
        self.records
            .iter()
            .enumerate()
            .map(|(index, record)| LegacyEntityInfo {
                index,
                id: record.id.clone(),
                marker: record.marker.clone(),
                known: !matches!(record.kind, RecordKind::Unknown),
            })
            .collect()
    }

    #[must_use]
    pub fn ice_count(&self, id: &DecoratorId) -> Option<u32> {
        self.records.iter().find_map(|record| {
            (record.id == id.as_str()).then_some(match record.kind {
                RecordKind::Ice { count } => Some(count),
                _ => None,
            })?
        })
    }

    #[must_use]
    pub fn glass_count(&self, id: &DecoratorId) -> Option<u32> {
        self.glass_source_record(id)
            .and_then(|record| match &record.kind {
                RecordKind::Glass { count } => Some(*count),
                _ => None,
            })
    }

    pub fn export(&self, snapshot: &LevelSnapshot) -> Result<String, LegacyError> {
        self.export_with_duration(snapshot, self.duration)
    }

    pub fn export_with_duration(
        &self,
        snapshot: &LevelSnapshot,
        duration: f32,
    ) -> Result<String, LegacyError> {
        if !duration.is_finite() || duration < 0.0 {
            return Err(invalid("dur", "must be a finite non-negative number"));
        }
        snapshot.validate_domain().map_err(domain)?;
        validate_legacy_semantics(snapshot)?;
        self.validate_opaque_tombstone_ids(snapshot)?;
        if matches!(self.compatibility, CompatibilityState::ReadOnly { .. })
            && (snapshot != &self.snapshot || duration != self.duration)
        {
            let CompatibilityState::ReadOnly { unknown_markers } = &self.compatibility else {
                unreachable!();
            };
            return Err(LegacyError::ReadOnlyUnknownMarkers {
                markers: unknown_markers.clone(),
            });
        }
        if snapshot == &self.snapshot && duration == self.duration {
            return Ok(self.source.render());
        }

        let mut root = self.source.clone();
        if duration != self.duration {
            root = root
                .set("dur", JsonNode::number(duration))
                .map_err(json_internal)?;
        }
        if snapshot.size() != self.snapshot.size() {
            root = root
                .set(
                    "bes",
                    JsonNode::array(vec![
                        JsonNode::number(snapshot.size().width()),
                        JsonNode::number(snapshot.size().height()),
                    ]),
                )
                .map_err(json_internal)?;
        }
        if snapshot.size() != self.snapshot.size() || snapshot.cells() != self.snapshot.cells() {
            let encoded = encode_mask(
                snapshot.cells().iter().map(|cell| *cell == CellKind::Floor),
                &[],
            )?;
            root = root
                .set("bdat", JsonNode::string(encoded))
                .map_err(json_internal)?;
        }
        if snapshot != &self.snapshot {
            let entities = self.build_entities(snapshot)?;
            root = root
                .set(ENTITIES_PROPERTY, JsonNode::array(entities))
                .map_err(json_internal)?;
            if self.glass_tombstones_changed(snapshot) {
                root = self.patch_glass_tombstones(root, self.build_tombstones(snapshot)?)?;
            }
        }
        Ok(root.render())
    }

    fn build_entities(&self, snapshot: &LevelSnapshot) -> Result<Vec<JsonNode>, LegacyError> {
        let entities: HashMap<_, _> = snapshot
            .entities()
            .iter()
            .map(|entity| (entity.id().as_str(), entity))
            .collect();
        let decorators: HashMap<_, _> = snapshot
            .decorators()
            .iter()
            .map(|decorator| (decorator.id().as_str(), decorator))
            .collect();
        let mut emitted = HashSet::new();
        let mut output = Vec::new();

        for record in &self.records {
            match &record.kind {
                RecordKind::Unknown => {
                    if entities.contains_key(record.id.as_str())
                        || decorators.contains_key(record.id.as_str())
                    {
                        return Err(LegacyError::DuplicateEntityId(record.id.clone()));
                    }
                    output.push(record.envelope.clone());
                }
                RecordKind::Block { .. } | RecordKind::Blind { .. } => {
                    if decorators.contains_key(record.id.as_str()) {
                        return Err(LegacyError::UnsupportedChange {
                            entity_id: record.id.clone(),
                            reason: "changing a placeable into a decorator would discard marker-specific payload data"
                                .to_owned(),
                        });
                    }
                    let Some(entity) = entities.get(record.id.as_str()) else {
                        continue;
                    };
                    emitted.insert(record.id.clone());
                    output.push(self.build_placeable(record, entity)?);
                }
                RecordKind::Ice { .. }
                | RecordKind::Glass { .. }
                | RecordKind::Direction
                | RecordKind::KeyLocker => {
                    if entities.contains_key(record.id.as_str()) {
                        return Err(LegacyError::UnsupportedChange {
                            entity_id: record.id.clone(),
                            reason: "changing a decorator into a placeable would discard marker-specific payload data"
                                .to_owned(),
                        });
                    }
                    let Some(decorator) = decorators.get(record.id.as_str()) else {
                        continue;
                    };
                    if is_glass_tombstone(decorator) {
                        if matches!(&record.kind, RecordKind::Glass { .. }) {
                            continue;
                        }
                        return Err(LegacyError::UnsupportedChange {
                            entity_id: record.id.clone(),
                            reason: "changing a decorator marker would discard marker-specific payload data"
                                .to_owned(),
                        });
                    }
                    emitted.insert(record.id.clone());
                    output.push(self.build_decorator(record, decorator)?);
                }
            }
        }
        for entity in snapshot.entities() {
            if emitted.insert(entity.id().as_str().to_owned()) {
                output.push(self.build_new_placeable(entity)?);
            }
        }
        for decorator in snapshot.decorators() {
            if !is_glass_tombstone(decorator) && emitted.insert(decorator.id().as_str().to_owned())
            {
                let envelope = if matches!(decorator.kind(), DecoratorKind::Glass { .. }) {
                    if let Some(record) = self.glass_source_record(decorator.id()) {
                        self.build_decorator(record, decorator)?
                    } else {
                        self.build_new_decorator(decorator)?
                    }
                } else {
                    self.build_new_decorator(decorator)?
                };
                output.push(envelope);
            }
        }
        Ok(output)
    }

    fn validate_opaque_tombstone_ids(&self, snapshot: &LevelSnapshot) -> Result<(), LegacyError> {
        for id in self
            .tombstones
            .iter()
            .filter_map(|tombstone| match tombstone {
                TombstoneRecord::Opaque { id, .. } => id.as_deref(),
                TombstoneRecord::Glass(_) => None,
            })
        {
            if snapshot
                .entities()
                .iter()
                .any(|entity| entity.id().as_str() == id)
                || snapshot
                    .decorators()
                    .iter()
                    .any(|decorator| decorator.id().as_str() == id)
            {
                return Err(LegacyError::DuplicateEntityId(id.to_owned()));
            }
        }
        Ok(())
    }

    fn glass_tombstones_changed(&self, snapshot: &LevelSnapshot) -> bool {
        let baseline: Vec<_> = self
            .snapshot
            .decorators()
            .iter()
            .filter(|decorator| is_glass_tombstone(decorator))
            .collect();
        let current: Vec<_> = snapshot
            .decorators()
            .iter()
            .filter(|decorator| is_glass_tombstone(decorator))
            .collect();
        baseline != current
    }

    fn glass_source_record(&self, id: &DecoratorId) -> Option<&LegacyRecord> {
        self.records
            .iter()
            .find(|record| {
                record.id == id.as_str() && matches!(&record.kind, RecordKind::Glass { .. })
            })
            .or_else(|| {
                self.tombstones
                    .iter()
                    .find_map(|tombstone| match tombstone {
                        TombstoneRecord::Glass(record)
                            if record.id == id.as_str()
                                && matches!(&record.kind, RecordKind::Glass { .. }) =>
                        {
                            Some(record)
                        }
                        TombstoneRecord::Glass(_) | TombstoneRecord::Opaque { .. } => None,
                    })
            })
    }

    fn build_tombstones(&self, snapshot: &LevelSnapshot) -> Result<Vec<JsonNode>, LegacyError> {
        let decorators: HashMap<_, _> = snapshot
            .decorators()
            .iter()
            .map(|decorator| (decorator.id().as_str(), decorator))
            .collect();
        let mut emitted = HashSet::new();
        let mut output = Vec::new();
        for tombstone in &self.tombstones {
            match tombstone {
                TombstoneRecord::Opaque { envelope, .. } => output.push(envelope.clone()),
                TombstoneRecord::Glass(record) => {
                    let Some(decorator) = decorators.get(record.id.as_str()) else {
                        continue;
                    };
                    if !matches!(decorator.kind(), DecoratorKind::Glass { .. }) {
                        return Err(LegacyError::UnsupportedChange {
                            entity_id: record.id.clone(),
                            reason: "changing a decorator marker would discard marker-specific payload data"
                                .to_owned(),
                        });
                    }
                    if !is_glass_tombstone(decorator) {
                        continue;
                    }
                    emitted.insert(record.id.clone());
                    output.push(self.build_decorator(record, decorator)?);
                }
            }
        }
        for record in &self.records {
            if !matches!(&record.kind, RecordKind::Glass { .. }) {
                continue;
            }
            let Some(decorator) = decorators.get(record.id.as_str()) else {
                continue;
            };
            if is_glass_tombstone(decorator) && emitted.insert(record.id.clone()) {
                output.push(self.build_decorator(record, decorator)?);
            }
        }
        for decorator in snapshot.decorators() {
            if is_glass_tombstone(decorator) && emitted.insert(decorator.id().as_str().to_owned()) {
                output.push(self.build_new_decorator(decorator)?);
            }
        }
        Ok(output)
    }

    fn patch_glass_tombstones(
        &self,
        root: JsonNode,
        tombstones: Vec<JsonNode>,
    ) -> Result<JsonNode, LegacyError> {
        if !self.tombstone_sidecar_writable {
            return Err(LegacyError::UnsupportedChange {
                entity_id: "_led".to_owned(),
                reason: "cannot change Glass tombstones because _led.dt is opaque".to_owned(),
            });
        }
        let tombstones = JsonNode::array(tombstones);
        let sidecar = match root.get("_led") {
            Some(existing) => {
                if existing.as_object().is_none() {
                    return Err(LegacyError::UnsupportedChange {
                        entity_id: "_led".to_owned(),
                        reason: "cannot add decorator tombstones because _led is not an object"
                            .to_owned(),
                    });
                }
                existing.set("dt", tombstones).map_err(json_internal)?
            }
            None => JsonNode::object(vec![("dt", tombstones)]),
        };
        root.set("_led", sidecar).map_err(json_internal)
    }

    fn build_placeable(
        &self,
        record: &LegacyRecord,
        entity: &PlaceableEntity,
    ) -> Result<JsonNode, LegacyError> {
        let baseline = self.snapshot.entity(entity.id()).ok_or_else(|| {
            LegacyError::Internal("source entity is absent from baseline".to_owned())
        })?;
        if entity == baseline {
            return Ok(record.envelope.clone());
        }
        match (&record.kind, entity.kind(), baseline.kind()) {
            (
                RecordKind::Block { metadata },
                PlaceableEntityKind::Block(block),
                PlaceableEntityKind::Block(old_block),
            ) => self.patch_block(record, entity, block, baseline, old_block, metadata),
            (
                RecordKind::Blind {
                    metadata,
                    wire_pixels,
                    ..
                },
                PlaceableEntityKind::Blind(blind),
                PlaceableEntityKind::Blind(old_blind),
            ) => self.patch_blind(
                record,
                entity,
                blind,
                baseline,
                old_blind,
                metadata,
                wire_pixels,
            ),
            _ => Err(LegacyError::UnsupportedChange {
                entity_id: record.id.clone(),
                reason:
                    "changing a legacy entity marker would discard marker-specific payload data"
                        .to_owned(),
            }),
        }
    }

    fn patch_block(
        &self,
        record: &LegacyRecord,
        entity: &PlaceableEntity,
        block: &Block,
        baseline: &PlaceableEntity,
        old_block: &Block,
        metadata: &GridMetadata,
    ) -> Result<JsonNode, LegacyError> {
        let layers_changed = block.collect_layers() != old_block.collect_layers();
        let locks: Vec<_> = block
            .collect_layers()
            .iter()
            .map(CollectLayer::is_locked)
            .collect();
        let old_locks: Vec<_> = old_block
            .collect_layers()
            .iter()
            .map(CollectLayer::is_locked)
            .collect();
        let metadata_count_changed = matches!(metadata, GridMetadata::SbclV2 { locks } if locks.len() != block.collect_layers().len());
        let data_changed = entity.shape() != baseline.shape()
            || locks != old_locks
            || metadata_count_changed
            || (layers_changed && matches!(metadata, GridMetadata::SbclV1));
        let rect_changed = entity.origin() != baseline.origin()
            || entity.shape().width() != baseline.shape().width()
            || entity.shape().height() != baseline.shape().height();
        let mut payload = record.payload.clone();
        if rect_changed || data_changed {
            payload = patch_grid(
                &payload,
                entity,
                rect_changed,
                data_changed,
                if data_changed { Some(&locks) } else { None },
                metadata,
                layers_changed,
            )?;
        }
        if layers_changed {
            let cc = build_collect_rows(
                block.collect_layers(),
                old_block.collect_layers(),
                record.payload.get("cc"),
            )?;
            payload = payload.set("cc", cc).map_err(json_internal)?;
        }
        Ok(envelope("block", payload))
    }

    #[allow(clippy::too_many_arguments)]
    fn patch_blind(
        &self,
        record: &LegacyRecord,
        entity: &PlaceableEntity,
        blind: &Blind,
        baseline: &PlaceableEntity,
        old_blind: &Blind,
        metadata: &GridMetadata,
        wire_pixels: &[u8],
    ) -> Result<JsonNode, LegacyError> {
        let shape_changed = entity.shape() != baseline.shape();
        let resolution_changed = blind.pixels_per_cell() != old_blind.pixels_per_cell();
        if blind.guides() != old_blind.guides() {
            return Err(LegacyError::UnsupportedMetadataChange {
                entity_id: record.id.clone(),
                metadata: "SBLN",
                reason: "encoding changed Blind guide-line topology is deferred".to_owned(),
            });
        }
        if matches!(metadata, GridMetadata::Sbln { .. }) && (shape_changed || resolution_changed) {
            return Err(LegacyError::UnsupportedMetadataChange {
                entity_id: record.id.clone(),
                metadata: "SBLN",
                reason: "the core model cannot transform Blind guide-line topology".to_owned(),
            });
        }
        let rect_changed = entity.origin() != baseline.origin()
            || entity.shape().width() != baseline.shape().width()
            || entity.shape().height() != baseline.shape().height();
        let mut payload = record.payload.clone();
        if rect_changed || shape_changed {
            payload = patch_grid(
                &payload,
                entity,
                rect_changed,
                shape_changed,
                None,
                metadata,
                false,
            )?;
        }
        if blind.tiles() != old_blind.tiles() || resolution_changed || shape_changed {
            payload = patch_canvas(
                &payload,
                entity.shape(),
                blind,
                baseline.shape(),
                old_blind,
                wire_pixels,
                &record.id,
            )?;
        }
        payload = patch_pool_boundary(&payload, blind.boundary(), old_blind.boundary())?;
        Ok(envelope("pool", payload))
    }

    fn build_decorator(
        &self,
        record: &LegacyRecord,
        decorator: &Decorator,
    ) -> Result<JsonNode, LegacyError> {
        let baseline = self
            .snapshot
            .decorators()
            .iter()
            .find(|candidate| candidate.id() == decorator.id())
            .ok_or_else(|| {
                LegacyError::Internal("source decorator is absent from baseline".to_owned())
            })?;
        if decorator == baseline {
            return Ok(record.envelope.clone());
        }
        let expected = marker_for_decorator(decorator);
        if expected != record.marker {
            return Err(LegacyError::UnsupportedChange {
                entity_id: record.id.clone(),
                reason: "changing a decorator marker would discard marker-specific payload data"
                    .to_owned(),
            });
        }
        let payload = patch_decorator_payload(record.payload.clone(), decorator, false)?;
        Ok(envelope(expected, payload))
    }

    fn build_new_placeable(&self, entity: &PlaceableEntity) -> Result<JsonNode, LegacyError> {
        let mut payload = JsonNode::object(vec![("eid", JsonNode::string(entity.id().as_str()))]);
        let locks = match entity.kind() {
            PlaceableEntityKind::Block(block) => block
                .collect_layers()
                .iter()
                .map(CollectLayer::is_locked)
                .collect::<Vec<_>>(),
            PlaceableEntityKind::Blind(_) => Vec::new(),
        };
        payload = patch_grid(
            &payload,
            entity,
            true,
            true,
            match entity.kind() {
                PlaceableEntityKind::Block(_) => Some(locks.as_slice()),
                PlaceableEntityKind::Blind(_) => None,
            },
            &GridMetadata::None,
            true,
        )?;
        match entity.kind() {
            PlaceableEntityKind::Block(block) => {
                payload = payload
                    .set("cc", build_collect_rows(block.collect_layers(), &[], None)?)
                    .map_err(json_internal)?;
                Ok(envelope("block", payload))
            }
            PlaceableEntityKind::Blind(blind) => {
                if !blind.guides().is_empty() {
                    return Err(LegacyError::UnsupportedMetadataChange {
                        entity_id: entity.id().as_str().to_owned(),
                        metadata: "SBLN",
                        reason: "encoding new Blind guide-line topology is deferred".to_owned(),
                    });
                }
                payload = patch_canvas(
                    &payload,
                    entity.shape(),
                    blind,
                    entity.shape(),
                    blind,
                    &[],
                    entity.id().as_str(),
                )?;
                payload = patch_pool_boundary(&payload, blind.boundary(), PoolBoundary::default())?;
                Ok(envelope("pool", payload))
            }
        }
    }

    fn build_new_decorator(&self, decorator: &Decorator) -> Result<JsonNode, LegacyError> {
        let payload = JsonNode::object(vec![("eid", JsonNode::string(decorator.id().as_str()))]);
        let payload = patch_decorator_payload(payload, decorator, true)?;
        Ok(envelope(marker_for_decorator(decorator), payload))
    }
}

fn is_glass_tombstone(decorator: &Decorator) -> bool {
    matches!(
        decorator.kind(),
        DecoratorKind::Glass {
            blocking_count: 0,
            ..
        }
    )
}

#[must_use]
pub const fn resolved_collect_radius(layer: &CollectLayer) -> u8 {
    match layer.radius() {
        Some(radius) => radius,
        None => DEFAULT_COLLECT_RADIUS_PIXELS,
    }
}

#[derive(Clone, Debug)]
struct PendingDecorator {
    id: String,
    kind: PendingDecoratorKind,
}

#[derive(Clone, Debug)]
enum PendingDecoratorKind {
    Ice {
        target: String,
        count: u32,
    },
    Glass {
        target: String,
        count: u32,
    },
    Direction {
        target: String,
        direction: CardinalDirection,
    },
    KeyLocker {
        key: String,
        locker: String,
    },
}

fn validate_codec_markers(codecs: &[&dyn UnknownEntityCodec]) -> Result<(), LegacyError> {
    let mut markers = HashSet::new();
    for codec in codecs {
        let marker = codec.marker();
        if marker.is_empty() {
            return Err(LegacyError::Plugin {
                marker: String::new(),
                message: "codec marker must be non-empty".to_owned(),
            });
        }
        if !markers.insert(marker) {
            return Err(LegacyError::Plugin {
                marker: marker.to_owned(),
                message: "more than one codec handles this marker".to_owned(),
            });
        }
    }
    Ok(())
}

fn required<'a>(object: &'a JsonNode, name: &str) -> Result<&'a JsonNode, LegacyError> {
    object.get(name).ok_or_else(|| invalid(name, "is required"))
}

fn parse_duration(node: &JsonNode) -> Result<f32, LegacyError> {
    let value = node
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= f64::from(f32::MAX))
        .ok_or_else(|| invalid("dur", "must be a finite non-negative number"))?;
    Ok(value as f32)
}

fn parse_board_size(node: &JsonNode) -> Result<GridSize, LegacyError> {
    let values = exact_array(node, 2, "bes")?;
    let width = parse_u16(&values[0], "bes[0]")?;
    let height = parse_u16(&values[1], "bes[1]")?;
    GridSize::new(width, height).map_err(domain)
}

fn parse_board_cells(node: &JsonNode, size: GridSize) -> Result<Vec<CellKind>, LegacyError> {
    let encoded = node
        .as_str()
        .ok_or_else(|| invalid("bdat", "must be a DataCodec string"))?;
    let decoded = DataCodec::decode(encoded).map_err(|source| codec_error("bdat", source))?;
    unpack_mask(&decoded.data, size.cell_count(), "bdat").map(|mask| {
        mask.into_iter()
            .map(|floor| {
                if floor {
                    CellKind::Floor
                } else {
                    CellKind::Wall
                }
            })
            .collect()
    })
}

fn parse_envelope<'a>(
    envelope: &'a JsonNode,
    path: &str,
) -> Result<(String, &'a JsonNode), LegacyError> {
    let values = exact_array(envelope, 2, path)?;
    let marker = parse_nonempty_string(&values[0], &format!("{path}[0]"))?;
    let payload = values[1]
        .as_object()
        .ok_or_else(|| invalid(format!("{path}[1]"), "must be an object"))?;
    Ok((marker, payload))
}

fn opaque_tombstone_id(envelope: &JsonNode) -> Option<String> {
    envelope
        .as_array()?
        .get(1)?
        .as_object()?
        .get("eid")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

fn parse_glass_tombstones(
    source: &JsonNode,
    ids: &mut HashSet<String>,
    pending_decorators: &mut Vec<PendingDecorator>,
) -> Result<(Vec<TombstoneRecord>, bool), LegacyError> {
    let Some(sidecar) = source.get("_led") else {
        return Ok((Vec::new(), true));
    };
    let Some(sidecar) = sidecar.as_object() else {
        return Ok((Vec::new(), false));
    };
    let Some(tombstones) = sidecar.get("dt") else {
        return Ok((Vec::new(), true));
    };
    let Some(tombstones) = tombstones.as_array() else {
        return Ok((Vec::new(), false));
    };
    let mut records = Vec::with_capacity(tombstones.len());
    for (index, envelope) in tombstones.iter().enumerate() {
        let path = format!("_led.dt[{index}]");
        let is_glass = envelope
            .as_array()
            .and_then(|values| values.first())
            .and_then(JsonNode::as_str)
            == Some("glass");
        if !is_glass {
            let id = opaque_tombstone_id(envelope);
            if let Some(id) = &id
                && !ids.insert(id.clone())
            {
                return Err(LegacyError::DuplicateEntityId(id.clone()));
            }
            records.push(TombstoneRecord::Opaque {
                id,
                envelope: envelope.clone(),
            });
            continue;
        }
        let (marker, payload) = parse_envelope(envelope, &path)?;
        let id = parse_nonempty_string(required(payload, "eid")?, &format!("{path}[1].eid"))?;
        if !ids.insert(id.clone()) {
            return Err(LegacyError::DuplicateEntityId(id));
        }
        let target = parse_decorator_target(payload, &path, "deco")?;
        let count = parse_u32(required(payload, "count")?, &format!("{path}[1].count"))?;
        if count != 0 {
            return Err(invalid(
                format!("{path}[1].count"),
                "Glass tombstone count must be zero",
            ));
        }
        pending_decorators.push(PendingDecorator {
            id: id.clone(),
            kind: PendingDecoratorKind::Glass { target, count },
        });
        records.push(TombstoneRecord::Glass(LegacyRecord {
            id,
            marker,
            envelope: envelope.clone(),
            payload: payload.clone(),
            kind: RecordKind::Glass { count },
        }));
    }
    Ok((records, true))
}

fn parse_grid(
    payload: &JsonNode,
    entity_path: &str,
    blind: bool,
) -> Result<(GridPoint, Shape, GridMetadata), LegacyError> {
    let path = format!("{entity_path}[1].g");
    let grid = required(payload, "g")?
        .as_object()
        .ok_or_else(|| invalid(&path, "must be an object"))?;
    let rect = exact_array(required(grid, "r")?, 4, &format!("{path}.r"))?;
    let x = parse_u16(&rect[0], &format!("{path}.r[0]"))?;
    let y = parse_u16(&rect[1], &format!("{path}.r[1]"))?;
    let width = parse_u8(&rect[2], &format!("{path}.r[2]"))?;
    let height = parse_u8(&rect[3], &format!("{path}.r[3]"))?;
    if width == 0 || height == 0 || u16::from(width) * u16::from(height) > 64 {
        return Err(invalid(
            format!("{path}.r"),
            "shape bounding area must be in 1..=64",
        ));
    }
    if !blind && (width > 8 || height > 8) {
        return Err(invalid(
            format!("{path}.r"),
            "Block shape dimensions must not exceed 8 x 8",
        ));
    }
    let encoded = required(grid, "d")?
        .as_str()
        .ok_or_else(|| invalid(format!("{path}.d"), "must be a DataCodec string"))?;
    let decoded =
        decode_grid(encoded).map_err(|source| codec_error(format!("{path}.d"), source))?;
    let metadata = parse_grid_metadata(&decoded.salt, blind, &format!("{path}.d"))?;
    let area = usize::from(width) * usize::from(height);
    let mask = unpack_mask(&decoded.data, area, &format!("{path}.d"))?;
    let occupied_mask = mask
        .iter()
        .enumerate()
        .fold(0_u64, |bits, (index, occupied)| {
            bits | (u64::from(*occupied) << index)
        });
    let shape = Shape::new(width, height, occupied_mask).map_err(domain)?;
    Ok((GridPoint::new(x, y), shape, metadata))
}

fn parse_grid_metadata(salt: &[u8], blind: bool, path: &str) -> Result<GridMetadata, LegacyError> {
    if salt.len() == SALT_SIZE {
        return Ok(GridMetadata::None);
    }
    let suffix = &salt[SALT_SIZE..];
    if blind {
        if suffix.len() < SBLN_HEADER_SIZE {
            return Err(invalid(path, "SBLN metadata header is truncated"));
        }
        if &suffix[..4] != b"SBLN" {
            return Err(invalid(
                path,
                "extended Blind salt has an unsupported marker",
            ));
        }
        if suffix[4] != 1 {
            return Err(invalid(
                path,
                format!("SBLN metadata version {} is unsupported", suffix[4]),
            ));
        }
        let width = u16::from_le_bytes([suffix[5], suffix[6]]);
        let height = u16::from_le_bytes([suffix[7], suffix[8]]);
        let width_usize = usize::from(width);
        let height_usize = usize::from(height);
        if width == 0
            || height == 0
            || width_usize > 64 * 32
            || height_usize > 64 * 32
            || width_usize * height_usize > 64 * 32 * 32
        {
            return Err(invalid(path, "SBLN dimensions exceed the supported budget"));
        }
        let horizontal = width_usize * (height_usize + 1);
        let vertical = (width_usize + 1) * height_usize;
        validate_packed_suffix(
            suffix,
            SBLN_HEADER_SIZE,
            horizontal + vertical,
            path,
            "SBLN",
        )?;
        return Ok(GridMetadata::Sbln { width, height });
    }

    if suffix.len() < 5 || &suffix[..4] != b"SBCL" {
        return Err(invalid(
            path,
            "extended Block salt has an unsupported marker",
        ));
    }
    match suffix[4] {
        1 => {
            if suffix.len() != 5 {
                return Err(invalid(
                    path,
                    "SBCL v1 metadata must contain exactly 5 bytes",
                ));
            }
            Ok(GridMetadata::SbclV1)
        }
        2 => {
            if suffix.len() < SBCL_V2_HEADER_SIZE {
                return Err(invalid(path, "SBCL v2 metadata header is truncated"));
            }
            let count = usize::from(u16::from_le_bytes([suffix[5], suffix[6]]));
            validate_packed_suffix(suffix, SBCL_V2_HEADER_SIZE, count, path, "SBCL v2")?;
            let locks = (0..count)
                .map(|index| suffix[SBCL_V2_HEADER_SIZE + index / 8] & (1 << (index % 8)) != 0)
                .collect();
            Ok(GridMetadata::SbclV2 { locks })
        }
        version => Err(invalid(
            path,
            format!("SBCL metadata version {version} is unsupported"),
        )),
    }
}

fn validate_packed_suffix(
    suffix: &[u8],
    header_len: usize,
    bit_count: usize,
    path: &str,
    label: &str,
) -> Result<(), LegacyError> {
    let byte_count = bit_count.div_ceil(8);
    if suffix.len() != header_len + byte_count {
        return Err(invalid(
            path,
            format!("{label} length does not match its declared size"),
        ));
    }
    if let Some(last) = suffix.last()
        && bit_count % 8 != 0
        && *last & !((1_u8 << (bit_count % 8)) - 1) != 0
    {
        return Err(invalid(path, format!("{label} has non-zero padding bits")));
    }
    Ok(())
}

fn parse_collect_layers(
    node: Option<&JsonNode>,
    metadata: &GridMetadata,
    path: &str,
) -> Result<Vec<CollectLayer>, LegacyError> {
    let rows: &[JsonNode] = match node {
        None => &[],
        Some(node) if node.is_null() => &[],
        Some(node) => node
            .as_array()
            .ok_or_else(|| invalid(format!("{path}[1].cc"), "must be an array or null"))?,
    };
    if rows.len() > MAX_COLLECT_ROWS {
        return Err(invalid(
            format!("{path}[1].cc"),
            "contains more than 65535 rows",
        ));
    }
    let locks = match metadata {
        GridMetadata::None => vec![false; rows.len()],
        GridMetadata::SbclV1 => {
            if rows.is_empty() {
                return Err(invalid(
                    format!("{path}[1].g.d"),
                    "SBCL v1 has no cc row to lock",
                ));
            }
            let mut locks = vec![false; rows.len()];
            locks[0] = true;
            locks
        }
        GridMetadata::SbclV2 { locks } => {
            if locks.len() != rows.len() {
                return Err(invalid(
                    format!("{path}[1].g.d"),
                    format!(
                        "SBCL v2 lock count {} does not match cc row count {}",
                        locks.len(),
                        rows.len()
                    ),
                ));
            }
            locks.clone()
        }
        GridMetadata::Sbln { .. } => {
            return Err(invalid(
                format!("{path}[1].g.d"),
                "SBLN metadata is not valid on a Block",
            ));
        }
    };

    rows.iter()
        .enumerate()
        .map(|(index, row)| parse_collect_row(row, locks[index], path, index))
        .collect()
}

fn parse_collect_row(
    node: &JsonNode,
    locked: bool,
    path: &str,
    index: usize,
) -> Result<CollectLayer, LegacyError> {
    let field = format!("{path}[1].cc[{index}]");
    if node.is_null() {
        return Ok(CollectLayer::new(
            DISABLED_COLLECT_COLOR_INDEX,
            None,
            CollectCapacity::Unlimited,
            locked,
        ));
    }
    let row = node
        .as_array()
        .ok_or_else(|| invalid(&field, "must be an array or null"))?;
    let match_index = match row.first() {
        None => -1,
        Some(value) if value.is_null() => -1,
        Some(value) => parse_i32(value, &format!("{field}[0]"))?,
    };
    let color_index = match match_index {
        -1 => DISABLED_COLLECT_COLOR_INDEX,
        0..=15 => match_index as u16,
        _ => {
            return Err(invalid(
                format!("{field}[0]"),
                "must be -1, null, or a color index in 0..=15",
            ));
        }
    };
    let radius = match row.get(1) {
        None => None,
        Some(value) if value.is_null() => None,
        Some(value) => Some(parse_u8(value, &format!("{field}[1]"))?),
    };
    let capacity = match row.get(2) {
        None => CollectCapacity::Unlimited,
        Some(value) if value.is_null() => CollectCapacity::Unlimited,
        Some(value) => match parse_i32(value, &format!("{field}[2]"))? {
            -1 => CollectCapacity::Unlimited,
            value @ 0.. => CollectCapacity::Finite(value as u32),
            _ => {
                return Err(invalid(
                    format!("{field}[2]"),
                    "must be -1, null, or non-negative",
                ));
            }
        },
    };
    if let Some(speed) = row.get(3)
        && !speed.is_null()
    {
        let value = speed
            .as_f64()
            .filter(|value| value.is_finite() && value.abs() <= f64::from(f32::MAX))
            .ok_or_else(|| invalid(format!("{field}[3]"), "must be null or finite"))?;
        let _ = value as f32;
    }
    Ok(CollectLayer::new(color_index, radius, capacity, locked))
}

fn parse_pool_boundary(payload: &JsonNode, path: &str) -> Result<PoolBoundary, LegacyError> {
    let stc = required(payload, "stc")?;
    Ok(PoolBoundary {
        padding_pixels: parse_boundary_pixels(stc.get("spp"), &format!("{path}[1].stc.spp"))?,
        corner_radius_pixels: parse_boundary_pixels(
            stc.get("spcr"),
            &format!("{path}[1].stc.spcr"),
        )?,
    })
}

fn parse_boundary_pixels(node: Option<&JsonNode>, path: &str) -> Result<Option<u16>, LegacyError> {
    let Some(node) = node.filter(|node| !node.is_null()) else {
        return Ok(None);
    };
    node.as_i64()
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value <= 1024)
        .map(Some)
        .ok_or_else(|| invalid(path, "must be null or an integer in 0..=1024"))
}

fn parse_canvas(
    payload: &JsonNode,
    shape: Shape,
    path: &str,
) -> Result<CanvasDescriptor, LegacyError> {
    let stc = required(payload, "stc")?
        .as_object()
        .ok_or_else(|| invalid(format!("{path}[1].stc"), "must be an object"))?;
    let canvas = required(stc, "c")?
        .as_object()
        .ok_or_else(|| invalid(format!("{path}[1].stc.c"), "must be an object"))?;
    let resolution = exact_array(required(canvas, "r")?, 2, &format!("{path}[1].stc.c.r"))?;
    let width = parse_positive_usize(&resolution[0], &format!("{path}[1].stc.c.r[0]"))?;
    let height = parse_positive_usize(&resolution[1], &format!("{path}[1].stc.c.r[1]"))?;
    if required(canvas, "cmp")?.as_bool() != Some(true) {
        return Err(invalid(format!("{path}[1].stc.c.cmp"), "must be true"));
    }
    let data = required(canvas, "data")?;
    let encoded = if data.is_null() {
        None
    } else {
        Some(
            data.as_str()
                .ok_or_else(|| {
                    invalid(
                        format!("{path}[1].stc.c.data"),
                        "must be a compressed DataCodec string or null",
                    )
                })?
                .to_owned(),
        )
    };
    let placeholder = width == 1 && height == 1 && encoded.is_none();
    if !placeholder {
        let shape_width = usize::from(shape.width());
        let shape_height = usize::from(shape.height());
        if width % shape_width != 0
            || height % shape_height != 0
            || width / shape_width != height / shape_height
            || !(1..=32).contains(&(width / shape_width))
        {
            return Err(invalid(
                format!("{path}[1].stc.c.r"),
                "must be a common 1..=32 pixels/cell multiple of the shape",
            ));
        }
    }
    Ok(CanvasDescriptor {
        width,
        height,
        encoded,
        placeholder,
    })
}

fn decode_blind(
    pending: &PendingBlind,
    resolution: usize,
) -> Result<(Blind, Vec<u8>), LegacyError> {
    let width = usize::from(pending.shape.width()) * resolution;
    let height = usize::from(pending.shape.height()) * resolution;
    if !pending.canvas.placeholder
        && (pending.canvas.width, pending.canvas.height) != (width, height)
    {
        return Err(invalid(
            format!("entity '{}' stc.c.r", pending.id),
            format!("does not match the shared {resolution} pixels/cell resolution"),
        ));
    }
    let wire = if let Some(encoded) = &pending.canvas.encoded {
        DataCodec::decode_gzip(encoded, width * height)
            .map_err(|source| codec_error(format!("entity '{}' stc.c.data", pending.id), source))?
            .data
    } else {
        vec![0; width * height]
    };
    let resolution_u8 = resolution as u8;
    let mut tiles = Vec::with_capacity(pending.shape.occupied_count() as usize);
    for cell in pending.shape.occupied_cells() {
        let mut colors = vec![0_u8; resolution * resolution];
        for pixel_y in 0..resolution {
            for pixel_x in 0..resolution {
                let local_x = usize::from(cell.x) * resolution + pixel_x;
                let local_y = usize::from(cell.y) * resolution + pixel_y;
                let wire_y = height - 1 - local_y;
                let value = wire[local_x + wire_y * width];
                if value == 0 {
                    continue;
                }
                let color = value >> 4;
                if !(1..=10).contains(&color) {
                    return Err(invalid(
                        format!("entity '{}' stc.c.data", pending.id),
                        format!("pixel ({local_x}, {wire_y}) has unsupported color index {color}"),
                    ));
                }
                colors[pixel_x + pixel_y * resolution] = color;
            }
        }
        tiles.push(BlindTile::from_colors(resolution_u8, colors).map_err(domain)?);
    }
    for local_y in 0..height {
        for local_x in 0..width {
            let cell_x = local_x / resolution;
            let cell_y = local_y / resolution;
            let occupied = pending
                .shape
                .contains(oreak_core::ShapeCell::new(cell_x as u8, cell_y as u8));
            let wire_y = height - 1 - local_y;
            if !occupied && wire[local_x + wire_y * width] != 0 {
                return Err(invalid(
                    format!("entity '{}' stc.c.data", pending.id),
                    "paints outside the irregular Blind footprint",
                ));
            }
        }
    }
    let blind = Blind::new(resolution_u8, tiles)
        .and_then(|blind| blind.with_boundary(pending.boundary))
        .map_err(domain)?;
    Ok((blind, wire))
}

fn resolve_decorators(
    placeables: &[PlaceableEntity],
    pending: Vec<PendingDecorator>,
) -> Result<Vec<Decorator>, LegacyError> {
    let blocks: HashSet<_> = placeables
        .iter()
        .filter_map(|entity| {
            matches!(entity.kind(), PlaceableEntityKind::Block(_))
                .then_some(entity.id().as_str().to_owned())
        })
        .collect();
    let blinds: HashSet<_> = placeables
        .iter()
        .filter_map(|entity| {
            matches!(entity.kind(), PlaceableEntityKind::Blind(_))
                .then_some(entity.id().as_str().to_owned())
        })
        .collect();
    let mut ice_targets = HashSet::new();
    let mut glass_targets = HashSet::new();
    let mut direction_targets = HashSet::new();
    let mut key_targets = HashSet::new();
    let mut locker_targets = HashSet::new();
    let mut decorators = Vec::with_capacity(pending.len());
    for decorator in pending {
        let value = match decorator.kind {
            PendingDecoratorKind::Ice { target, count } => {
                require_block(&blocks, &decorator.id, &target, "deco")?;
                if !ice_targets.insert(target.clone()) {
                    return Err(invalid(
                        decorator.id,
                        format!("Block '{target}' has more than one Ice decorator"),
                    ));
                }
                Decorator::ice_with_blocking_count(decorator.id, target, count)
            }
            PendingDecoratorKind::Glass { target, count } => {
                require_blind(&blinds, &decorator.id, &target, "deco")?;
                if !glass_targets.insert(target.clone()) {
                    return Err(invalid(
                        decorator.id,
                        format!("Pool '{target}' has more than one Glass decorator"),
                    ));
                }
                Decorator::glass_with_blocking_count(decorator.id, target, count)
            }
            PendingDecoratorKind::Direction { target, direction } => {
                require_block(&blocks, &decorator.id, &target, "deco")?;
                if !direction_targets.insert(target.clone()) {
                    return Err(invalid(
                        decorator.id,
                        format!("Block '{target}' has more than one Direction decorator"),
                    ));
                }
                Decorator::direction(decorator.id, target, direction)
            }
            PendingDecoratorKind::KeyLocker { key, locker } => {
                require_block(&blocks, &decorator.id, &key, "deco")?;
                require_block(&blocks, &decorator.id, &locker, "lock")?;
                if key == locker {
                    return Err(invalid(
                        decorator.id,
                        "Key and Locker Blocks must be distinct",
                    ));
                }
                if !key_targets.insert(key.clone()) {
                    return Err(invalid(
                        decorator.id,
                        format!("Key Block '{key}' has more than one Key & Locker relation"),
                    ));
                }
                if !locker_targets.insert(locker.clone()) {
                    return Err(invalid(
                        decorator.id,
                        format!("Locker Block '{locker}' has more than one active Key"),
                    ));
                }
                Decorator::key_locker(decorator.id, locker, key)
            }
        };
        decorators.push(value);
    }
    Ok(decorators)
}

fn require_block(
    blocks: &HashSet<String>,
    decorator: &str,
    target: &str,
    field: &str,
) -> Result<(), LegacyError> {
    if blocks.contains(target) {
        Ok(())
    } else {
        Err(LegacyError::InvalidReference {
            decorator_id: decorator.to_owned(),
            field: field.to_owned(),
            entity_id: target.to_owned(),
        })
    }
}

fn require_blind(
    blinds: &HashSet<String>,
    decorator: &str,
    target: &str,
    field: &str,
) -> Result<(), LegacyError> {
    if blinds.contains(target) {
        Ok(())
    } else {
        Err(LegacyError::InvalidReference {
            decorator_id: decorator.to_owned(),
            field: field.to_owned(),
            entity_id: target.to_owned(),
        })
    }
}

fn validate_legacy_semantics(snapshot: &LevelSnapshot) -> Result<(), LegacyError> {
    let mut ids: HashSet<&str> = snapshot
        .entities()
        .iter()
        .map(|entity| entity.id().as_str())
        .collect();
    for decorator in snapshot.decorators() {
        if !ids.insert(decorator.id().as_str()) {
            return Err(LegacyError::DuplicateEntityId(
                decorator.id().as_str().to_owned(),
            ));
        }
    }
    let blocks: HashSet<_> = snapshot
        .entities()
        .iter()
        .filter_map(|entity| {
            matches!(entity.kind(), PlaceableEntityKind::Block(_)).then_some(entity.id())
        })
        .collect();
    let blinds: HashSet<_> = snapshot
        .entities()
        .iter()
        .filter_map(|entity| {
            matches!(entity.kind(), PlaceableEntityKind::Blind(_)).then_some(entity.id())
        })
        .collect();
    let mut ice = HashSet::new();
    let mut glass = HashSet::new();
    let mut direction = HashSet::new();
    let mut keys = HashSet::new();
    let mut lockers = HashSet::new();
    for decorator in snapshot.decorators() {
        match decorator.kind() {
            DecoratorKind::Ice { entity, .. } => {
                validate_target(&blocks, decorator, entity, "deco")?;
                if decorator
                    .ice_blocking_count()
                    .is_some_and(|count| count > i32::MAX as u32)
                {
                    return Err(invalid(
                        decorator.id().as_str(),
                        "Ice blocking count exceeds the legacy signed 32-bit range",
                    ));
                }
                if !ice.insert(entity) {
                    return Err(invalid(
                        decorator.id().as_str(),
                        "duplicate Ice decorator on one Block",
                    ));
                }
            }
            DecoratorKind::Glass { entity, .. } => {
                validate_target(&blinds, decorator, entity, "deco")?;
                let count = decorator.glass_blocking_count().unwrap_or_default();
                if count > i32::MAX as u32 {
                    return Err(invalid(
                        decorator.id().as_str(),
                        "Glass blocking count exceeds the legacy signed 32-bit range",
                    ));
                }
                if !glass.insert(entity) {
                    return Err(invalid(
                        decorator.id().as_str(),
                        "duplicate Glass decorator on one Pool",
                    ));
                }
            }
            DecoratorKind::Direction {
                entity,
                direction: value,
            } => {
                validate_target(&blocks, decorator, entity, "deco")?;
                if matches!(value, CardinalDirection::Left | CardinalDirection::Down) {
                    return Err(LegacyError::UnsupportedChange {
                        entity_id: decorator.id().as_str().to_owned(),
                        reason: "legacy Direction stores only Horizontal/Vertical and cannot preserve cardinal sign"
                            .to_owned(),
                    });
                }
                if !direction.insert(entity) {
                    return Err(invalid(
                        decorator.id().as_str(),
                        "duplicate Direction decorator on one Block",
                    ));
                }
            }
            DecoratorKind::KeyLocker { entity, key } => {
                validate_target(&blocks, decorator, entity, "lock")?;
                validate_target(&blocks, decorator, key, "deco")?;
                if entity == key {
                    return Err(invalid(
                        decorator.id().as_str(),
                        "Key and Locker Blocks must be distinct",
                    ));
                }
                if !keys.insert(key) || !lockers.insert(entity) {
                    return Err(invalid(
                        decorator.id().as_str(),
                        "duplicate Key or Locker ownership",
                    ));
                }
            }
        }
    }
    for entity in snapshot.entities() {
        if let PlaceableEntityKind::Block(block) = entity.kind() {
            if block.collect_layers().len() > MAX_COLLECT_ROWS {
                return Err(invalid(
                    entity.id().as_str(),
                    "Block has more than 65535 collect rows",
                ));
            }
            for layer in block.collect_layers() {
                if layer.color_index() != DISABLED_COLLECT_COLOR_INDEX && layer.color_index() > 15 {
                    return Err(invalid(
                        entity.id().as_str(),
                        "collect color index must be disabled or in 0..=15",
                    ));
                }
                if matches!(layer.capacity(), CollectCapacity::Finite(value) if value > i32::MAX as u32)
                {
                    return Err(invalid(
                        entity.id().as_str(),
                        "collect capacity exceeds the legacy signed 32-bit range",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_target(
    blocks: &HashSet<&EntityId>,
    decorator: &Decorator,
    target: &EntityId,
    field: &str,
) -> Result<(), LegacyError> {
    if blocks.contains(target) {
        Ok(())
    } else {
        Err(LegacyError::InvalidReference {
            decorator_id: decorator.id().as_str().to_owned(),
            field: field.to_owned(),
            entity_id: target.as_str().to_owned(),
        })
    }
}

fn patch_grid(
    payload: &JsonNode,
    entity: &PlaceableEntity,
    patch_rect: bool,
    patch_data: bool,
    locks: Option<&[bool]>,
    source_metadata: &GridMetadata,
    collect_rows_changed: bool,
) -> Result<JsonNode, LegacyError> {
    let mut grid = payload
        .get("g")
        .cloned()
        .unwrap_or_else(|| JsonNode::object(Vec::new()));
    if patch_rect {
        grid = grid
            .set(
                "r",
                JsonNode::array(vec![
                    JsonNode::number(entity.origin().x),
                    JsonNode::number(entity.origin().y),
                    JsonNode::number(entity.shape().width()),
                    JsonNode::number(entity.shape().height()),
                ]),
            )
            .map_err(json_internal)?;
    }
    if patch_data {
        if matches!(source_metadata, GridMetadata::Sbln { .. }) {
            return Err(LegacyError::UnsupportedMetadataChange {
                entity_id: entity.id().as_str().to_owned(),
                metadata: "SBLN",
                reason: "changing the shape mask requires transforming guide-line metadata"
                    .to_owned(),
            });
        }
        let metadata = locks.map_or_else(Vec::new, |locks| {
            build_lock_metadata(locks, source_metadata, collect_rows_changed)
        });
        let encoded = encode_mask(
            (0..entity.shape().bounding_area())
                .map(|index| entity.shape().occupied_mask() & (1_u64 << u32::from(index)) != 0),
            &metadata,
        )?;
        grid = grid
            .set("d", JsonNode::string(encoded))
            .map_err(json_internal)?;
    }
    payload.set("g", grid).map_err(json_internal)
}

fn build_lock_metadata(
    locks: &[bool],
    source: &GridMetadata,
    collect_rows_changed: bool,
) -> Vec<u8> {
    if !locks.iter().any(|locked| *locked) {
        return Vec::new();
    }
    if matches!(source, GridMetadata::SbclV1)
        && !collect_rows_changed
        && locks.first() == Some(&true)
        && locks.iter().skip(1).all(|locked| !locked)
    {
        return b"SBCL\x01".to_vec();
    }
    let mut metadata = Vec::with_capacity(SBCL_V2_HEADER_SIZE + locks.len().div_ceil(8));
    metadata.extend_from_slice(b"SBCL");
    metadata.push(2);
    metadata.extend_from_slice(&(locks.len() as u16).to_le_bytes());
    metadata.resize(SBCL_V2_HEADER_SIZE + locks.len().div_ceil(8), 0);
    for (index, locked) in locks.iter().enumerate() {
        if *locked {
            metadata[SBCL_V2_HEADER_SIZE + index / 8] |= 1 << (index % 8);
        }
    }
    metadata
}

fn build_collect_rows(
    layers: &[CollectLayer],
    baseline: &[CollectLayer],
    source: Option<&JsonNode>,
) -> Result<JsonNode, LegacyError> {
    let source_rows = source.and_then(JsonNode::as_array).unwrap_or(&[]);
    let mut candidates: Vec<Option<&JsonNode>> = vec![None; layers.len()];
    let mut exact_raw = vec![false; layers.len()];
    let mut used = vec![false; source_rows.len()];
    for index in 0..layers.len() {
        if index < baseline.len() && index < source_rows.len() && layers[index] == baseline[index] {
            candidates[index] = Some(&source_rows[index]);
            exact_raw[index] = true;
            used[index] = true;
        }
    }
    for index in 0..layers.len() {
        if candidates[index].is_some() {
            continue;
        }
        if let Some(source_index) = baseline.iter().enumerate().find_map(|(candidate, layer)| {
            (candidate < source_rows.len() && !used[candidate] && *layer == layers[index])
                .then_some(candidate)
        }) {
            candidates[index] = Some(&source_rows[source_index]);
            exact_raw[index] = true;
            used[source_index] = true;
        }
    }
    for index in 0..layers.len() {
        if candidates[index].is_none()
            && index < baseline.len()
            && index < source_rows.len()
            && !used[index]
        {
            candidates[index] = Some(&source_rows[index]);
            used[index] = true;
        }
    }
    let rows = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| {
            if exact_raw[index]
                && let Some(raw) = candidates[index]
            {
                return raw.clone();
            }
            build_collect_row(layer, candidates[index])
        })
        .collect();
    Ok(JsonNode::array(rows))
}

fn build_collect_row(layer: &CollectLayer, candidate: Option<&JsonNode>) -> JsonNode {
    if layer.color_index() == DISABLED_COLLECT_COLOR_INDEX
        && layer.radius().is_none()
        && layer.capacity() == CollectCapacity::Unlimited
        && candidate.is_some_and(JsonNode::is_null)
    {
        return JsonNode::null();
    }
    let mut row = candidate
        .and_then(JsonNode::as_array)
        .map(<[JsonNode]>::to_vec)
        .unwrap_or_default();
    set_array_value(
        &mut row,
        0,
        JsonNode::number(if layer.color_index() == DISABLED_COLLECT_COLOR_INDEX {
            -1_i32
        } else {
            i32::from(layer.color_index())
        }),
    );
    set_array_value(
        &mut row,
        1,
        layer.radius().map_or_else(JsonNode::null, JsonNode::number),
    );
    set_array_value(
        &mut row,
        2,
        match layer.capacity() {
            CollectCapacity::Unlimited => JsonNode::number(-1),
            CollectCapacity::Finite(value) => JsonNode::number(value),
        },
    );
    JsonNode::array(row)
}

fn set_array_value(values: &mut Vec<JsonNode>, index: usize, value: JsonNode) {
    values.resize_with(index + 1, JsonNode::null);
    values[index] = value;
}

fn patch_pool_boundary(
    payload: &JsonNode,
    boundary: PoolBoundary,
    baseline: PoolBoundary,
) -> Result<JsonNode, LegacyError> {
    if boundary == baseline {
        return Ok(payload.clone());
    }
    let mut stc = payload
        .get("stc")
        .cloned()
        .unwrap_or_else(|| JsonNode::object(Vec::new()));
    for (field, value, old_value) in [
        ("spp", boundary.padding_pixels, baseline.padding_pixels),
        (
            "spcr",
            boundary.corner_radius_pixels,
            baseline.corner_radius_pixels,
        ),
    ] {
        // Leave unchanged absence, null, and explicit zero exactly as authored.
        if value != old_value {
            stc = stc
                .set(field, value.map_or_else(JsonNode::null, JsonNode::number))
                .map_err(json_internal)?;
        }
    }
    payload.set("stc", stc).map_err(json_internal)
}

#[allow(clippy::too_many_arguments)]
fn patch_canvas(
    payload: &JsonNode,
    shape: Shape,
    blind: &Blind,
    baseline_shape: Shape,
    baseline_blind: &Blind,
    baseline_wire: &[u8],
    entity_id: &str,
) -> Result<JsonNode, LegacyError> {
    let resolution = usize::from(blind.pixels_per_cell());
    let width = usize::from(shape.width()) * resolution;
    let height = usize::from(shape.height()) * resolution;
    let old_resolution = usize::from(baseline_blind.pixels_per_cell());
    let old_width = usize::from(baseline_shape.width()) * old_resolution;
    let old_height = usize::from(baseline_shape.height()) * old_resolution;
    let mut wire = vec![0_u8; width * height];
    for local_y in 0..height {
        for local_x in 0..width {
            let color = blind_color(blind, shape, local_x, local_y);
            if color == 0 {
                continue;
            }
            let old_color = if local_x < old_width && local_y < old_height {
                blind_color(baseline_blind, baseline_shape, local_x, local_y)
            } else {
                0
            };
            let preserved = if color == old_color && baseline_wire.len() == old_width * old_height {
                baseline_wire[local_x + (old_height - 1 - local_y) * old_width]
            } else {
                0
            };
            wire[local_x + (height - 1 - local_y) * width] = if preserved == 0 {
                (color << 4) | 12
            } else {
                preserved
            };
        }
    }
    let painted = wire.iter().any(|pixel| *pixel != 0);
    let data =
        if !painted && (width, height) != (1, 1) {
            JsonNode::null()
        } else {
            JsonNode::string(DataCodec::encode_gzip(&wire).map_err(|source| {
                codec_error(format!("entity '{entity_id}' stc.c.data"), source)
            })?)
        };
    let mut stc = payload
        .get("stc")
        .cloned()
        .unwrap_or_else(|| JsonNode::object(Vec::new()));
    let mut canvas = stc
        .get("c")
        .cloned()
        .unwrap_or_else(|| JsonNode::object(Vec::new()));
    canvas = canvas
        .set(
            "r",
            JsonNode::array(vec![JsonNode::number(width), JsonNode::number(height)]),
        )
        .and_then(|canvas| canvas.set("cmp", JsonNode::bool(true)))
        .and_then(|canvas| canvas.set("data", data))
        .map_err(json_internal)?;
    stc = stc.set("c", canvas).map_err(json_internal)?;
    payload.set("stc", stc).map_err(json_internal)
}

fn blind_color(blind: &Blind, shape: Shape, local_x: usize, local_y: usize) -> u8 {
    let resolution = usize::from(blind.pixels_per_cell());
    let cell =
        oreak_core::ShapeCell::new((local_x / resolution) as u8, (local_y / resolution) as u8);
    blind
        .tile_for_cell(shape, cell)
        .and_then(|tile| tile.color((local_x % resolution) as u8, (local_y % resolution) as u8))
        .unwrap_or_default()
}

fn patch_decorator_payload(
    mut payload: JsonNode,
    decorator: &Decorator,
    _new: bool,
) -> Result<JsonNode, LegacyError> {
    match decorator.kind() {
        DecoratorKind::Ice {
            entity,
            blocking_count,
        } => {
            payload = payload
                .set("deco", JsonNode::string(entity.as_str()))
                .map_err(json_internal)?;
            payload = payload
                .set("count", JsonNode::number(*blocking_count))
                .map_err(json_internal)?;
        }
        DecoratorKind::Glass {
            entity,
            blocking_count,
        } => {
            payload = payload
                .set("deco", JsonNode::string(entity.as_str()))
                .and_then(|payload| payload.set("count", JsonNode::number(*blocking_count)))
                .map_err(json_internal)?;
        }
        DecoratorKind::Direction { entity, direction } => {
            let axis = match direction {
                CardinalDirection::Left | CardinalDirection::Right => "Horizontal",
                CardinalDirection::Up | CardinalDirection::Down => "Vertical",
            };
            payload = payload
                .set("deco", JsonNode::string(entity.as_str()))
                .and_then(|payload| payload.set("dir", JsonNode::string(axis)))
                .map_err(json_internal)?;
        }
        DecoratorKind::KeyLocker { entity, key } => {
            payload = payload
                .set("deco", JsonNode::string(key.as_str()))
                .and_then(|payload| payload.set("lock", JsonNode::string(entity.as_str())))
                .map_err(json_internal)?;
        }
    }
    Ok(payload)
}

fn marker_for_decorator(decorator: &Decorator) -> &'static str {
    match decorator.kind() {
        DecoratorKind::Ice { .. } => "ice",
        DecoratorKind::Glass { .. } => "glass",
        DecoratorKind::Direction { .. } => "direction",
        DecoratorKind::KeyLocker { .. } => "key-locker",
    }
}

fn envelope(marker: &str, payload: JsonNode) -> JsonNode {
    JsonNode::array(vec![JsonNode::string(marker), payload])
}

fn encode_mask(
    values: impl IntoIterator<Item = bool>,
    metadata: &[u8],
) -> Result<String, LegacyError> {
    let values: Vec<_> = values.into_iter().collect();
    let mut bytes = vec![0_u8; values.len().div_ceil(8)];
    for (index, value) in values.into_iter().enumerate() {
        if value {
            bytes[index / 8] |= 1 << (index % 8);
        }
    }
    if metadata.is_empty() {
        DataCodec::encode(&bytes).map_err(|source| codec_error("DataCodec encode", source))
    } else {
        encode_with_metadata(&bytes, metadata)
            .map_err(|source| codec_error("DataCodec encode", source))
    }
}

fn unpack_mask(data: &[u8], bit_count: usize, path: &str) -> Result<Vec<bool>, LegacyError> {
    let expected = bit_count.div_ceil(8);
    if data.len() != expected {
        return Err(invalid(
            path,
            format!("encodes {} bytes; expected {expected}", data.len()),
        ));
    }
    if bit_count % 8 != 0
        && data
            .last()
            .is_some_and(|last| *last & !((1_u8 << (bit_count % 8)) - 1) != 0)
    {
        return Err(invalid(path, "contains non-zero padding bits"));
    }
    Ok((0..bit_count)
        .map(|index| data[index / 8] & (1 << (index % 8)) != 0)
        .collect())
}

fn exact_array<'a>(
    node: &'a JsonNode,
    len: usize,
    path: &str,
) -> Result<&'a [JsonNode], LegacyError> {
    node.as_array()
        .filter(|values| values.len() == len)
        .ok_or_else(|| invalid(path, format!("must be an array of exactly {len} items")))
}

fn parse_nonempty_string(node: &JsonNode, path: &str) -> Result<String, LegacyError> {
    node.as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(path, "must be a non-empty string"))
}

fn parse_decorator_target(
    payload: &JsonNode,
    path: &str,
    field: &str,
) -> Result<String, LegacyError> {
    parse_nonempty_string(required(payload, field)?, &format!("{path}[1].{field}"))
}

fn parse_i32(node: &JsonNode, path: &str) -> Result<i32, LegacyError> {
    node.as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| invalid(path, "must be a 32-bit integer"))
}

fn parse_u32(node: &JsonNode, path: &str) -> Result<u32, LegacyError> {
    node.as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid(path, "must be a non-negative signed 32-bit integer"))
}

fn parse_u16(node: &JsonNode, path: &str) -> Result<u16, LegacyError> {
    node.as_i64()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| invalid(path, "must be a non-negative 16-bit integer"))
}

fn parse_u8(node: &JsonNode, path: &str) -> Result<u8, LegacyError> {
    node.as_i64()
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| invalid(path, "must be an integer in 0..=255"))
}

fn parse_positive_usize(node: &JsonNode, path: &str) -> Result<usize, LegacyError> {
    node.as_i64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid(path, "must be a positive integer"))
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> LegacyError {
    LegacyError::InvalidField {
        path: path.into(),
        message: message.into(),
    }
}

fn codec_error(path: impl Into<String>, source: CodecError) -> LegacyError {
    LegacyError::DataCodec {
        path: path.into(),
        source,
    }
}

fn domain(error: impl std::fmt::Display) -> LegacyError {
    LegacyError::Domain(error.to_string())
}

fn json_internal(error: impl std::fmt::Display) -> LegacyError {
    LegacyError::Internal(error.to_string())
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LegacyError {
    #[error("invalid level JSON at byte {offset}: {message}")]
    Json { offset: usize, message: String },
    #[error("invalid legacy field '{path}': {message}")]
    InvalidField { path: String, message: String },
    #[error("invalid DataCodec at '{path}': {source}")]
    DataCodec { path: String, source: CodecError },
    #[error("duplicate global entity ID '{0}'")]
    DuplicateEntityId(String),
    #[error(
        "decorator '{decorator_id}' field '{field}' references a missing or incompatible entity '{entity_id}'"
    )]
    InvalidReference {
        decorator_id: String,
        field: String,
        entity_id: String,
    },
    #[error("invalid oreak-core level: {0}")]
    Domain(String),
    #[error("unknown entity markers require a plugin codec before editing: {markers:?}")]
    ReadOnlyUnknownMarkers { markers: Vec<String> },
    #[error("plugin codec for marker '{marker}' rejected the payload: {message}")]
    Plugin { marker: String, message: String },
    #[error("cannot change entity '{entity_id}': {reason}")]
    UnsupportedChange { entity_id: String, reason: String },
    #[error("cannot change entity '{entity_id}' with preserved {metadata} metadata: {reason}")]
    UnsupportedMetadataChange {
        entity_id: String,
        metadata: &'static str,
        reason: String,
    },
    #[error("legacy adapter internal error: {0}")]
    Internal(String),
}
