use std::collections::{BTreeMap, BTreeSet};

use oreak_core::{
    ActorId, ApplyOutcome, BlameEntry, Blind, BlindPixel, BlindStroke, BlindTile, Block, CellKind,
    CollectCapacity, CollectLayer, CommandEnvelope, CommandMetadata, Decorator, DecoratorId,
    DecoratorKind, DirectionMode, EntityId, EntityMove, GridAnchor, GridPoint, GridSize,
    HistoryEvent, LevelCommand, LevelSnapshot, LevelTarget, LevelTimeline, PlaceableEntity, Shape,
    ShapeCell, ShapeError, TimelineError,
};
use oreak_protocol::{LevelPresenceItem, PresenceId, PresenceParticipant, ProjectLevelTarget};

pub const DEFAULT_BLIND_PIXELS_PER_CELL: u8 = 32;

pub fn reconnect_delay_ms(failure_count: u32) -> u32 {
    let exponent = failure_count.saturating_sub(1).min(4);
    (1_000_u32 << exponent).min(15_000)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresenceRoster {
    self_id: Option<PresenceId>,
    participants: BTreeMap<PresenceId, PresenceParticipant>,
}

impl PresenceRoster {
    pub fn apply(&mut self, expected: &ProjectLevelTarget, item: LevelPresenceItem) -> bool {
        match item {
            LevelPresenceItem::Snapshot {
                target,
                self_id,
                participants,
            } if &target == expected => {
                self.self_id = Some(self_id);
                self.participants = participants
                    .into_iter()
                    .map(|participant| (participant.id.clone(), participant))
                    .collect();
                true
            }
            LevelPresenceItem::Joined {
                target,
                participant,
            } if &target == expected => {
                self.participants
                    .insert(participant.id.clone(), participant);
                true
            }
            LevelPresenceItem::Left {
                target,
                presence_id,
            } if &target == expected => self.participants.remove(&presence_id).is_some(),
            LevelPresenceItem::Cursor {
                target,
                presence_id,
                cursor,
            } if &target == expected => {
                self.participants
                    .get_mut(&presence_id)
                    .is_some_and(|participant| {
                        participant.cursor = cursor;
                        true
                    })
            }
            _ => false,
        }
    }

    pub fn clear(&mut self) {
        self.self_id = None;
        self.participants.clear();
    }

    #[must_use]
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    #[must_use]
    pub fn actor_count(&self) -> usize {
        self.participants
            .values()
            .map(|participant| participant.actor.clone())
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn participants(&self) -> impl Iterator<Item = &PresenceParticipant> {
        self.participants.values()
    }

    #[must_use]
    pub fn participant(&self, id: &PresenceId) -> Option<&PresenceParticipant> {
        self.participants.get(id)
    }

    #[must_use]
    pub fn is_self(&self, id: &PresenceId) -> bool {
        self.self_id.as_ref() == Some(id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Select,
    Map,
    Brush,
    Sandbox,
}

#[cfg(target_arch = "wasm32")]
impl Mode {
    pub const ALL: [Self; 4] = [Self::Select, Self::Map, Self::Brush, Self::Sandbox];

    pub const fn key(self) -> char {
        match self {
            Self::Select => 'Q',
            Self::Map => 'W',
            Self::Brush => 'B',
            Self::Sandbox => 'P',
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Map => "Map",
            Self::Brush => "Brush",
            Self::Sandbox => "Sandbox",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Select => "Inspect cells and core placeables with collaborative provenance",
            Self::Map => "Paint logical floor and wall cells",
            Self::Brush => "Paint, erase, or fill pixels on the selected Pool",
            Self::Sandbox => "Parity gated: source sandbox behavior is not available yet",
        }
    }

    pub const fn is_parity_gated(self) -> bool {
        matches!(self, Self::Sandbox)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutScope {
    Workspace,
    TextEntry,
    Palette,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    SelectMode(Mode),
    MoveSelection(MoveDirection),
    Undo,
    TogglePalette,
    CloseOverlay,
    PaletteSubmit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveDirection {
    Up,
    Right,
    Down,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapResizeEdge {
    Top,
    Right,
    Bottom,
    Left,
}

pub fn plan_edge_resize(
    size: GridSize,
    edge: MapResizeEdge,
    outward_cells: i32,
) -> (GridSize, GridAnchor) {
    let (width, height, anchor) = match edge {
        MapResizeEdge::Top => (
            i32::from(size.width()),
            (i32::from(size.height()) + outward_cells).clamp(1, 256),
            GridAnchor::Top,
        ),
        MapResizeEdge::Right => (
            (i32::from(size.width()) + outward_cells).clamp(1, 256),
            i32::from(size.height()),
            GridAnchor::Left,
        ),
        MapResizeEdge::Bottom => (
            i32::from(size.width()),
            (i32::from(size.height()) + outward_cells).clamp(1, 256),
            GridAnchor::Bottom,
        ),
        MapResizeEdge::Left => (
            (i32::from(size.width()) + outward_cells).clamp(1, 256),
            i32::from(size.height()),
            GridAnchor::Right,
        ),
    };
    (
        GridSize::new(width as u16, height as u16)
            .expect("edge resize clamps dimensions to core bounds"),
        anchor,
    )
}

pub fn plan_content_aware_edge_resize(
    snapshot: &LevelSnapshot,
    edge: MapResizeEdge,
    outward_cells: i32,
) -> (GridSize, GridAnchor) {
    let size = snapshot.size();
    let (planned, anchor) = plan_edge_resize(size, edge, outward_cells);
    let mut min_x = size.width();
    let mut min_y = size.height();
    let mut max_x = 0;
    let mut max_y = 0;
    let mut has_content = false;
    let mut include = |point: GridPoint| {
        has_content = true;
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    };

    for (index, kind) in snapshot.cells().iter().enumerate() {
        if *kind != CellKind::Floor {
            let width = usize::from(size.width());
            include(GridPoint::new(
                u16::try_from(index % width).expect("cell x fits the grid axis"),
                u16::try_from(index / width).expect("cell y fits the grid axis"),
            ));
        }
    }
    for entity in snapshot.entities() {
        if let Some(points) = shape_world_points(entity.origin(), entity.shape()) {
            points.into_iter().for_each(&mut include);
        }
    }

    if !has_content {
        return (planned, anchor);
    }
    let (width, height) = match edge {
        MapResizeEdge::Top => (planned.width(), planned.height().max(size.height() - min_y)),
        MapResizeEdge::Right => (planned.width().max(max_x + 1), planned.height()),
        MapResizeEdge::Bottom => (planned.width(), planned.height().max(max_y + 1)),
        MapResizeEdge::Left => (planned.width().max(size.width() - min_x), planned.height()),
    };
    (
        GridSize::new(width, height).expect("content bounds stay within the current grid"),
        anchor,
    )
}

impl MoveDirection {
    pub const fn offset(self) -> (i16, i16) {
        match self {
            Self::Up => (0, -1),
            Self::Right => (1, 0),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Cell(GridPoint),
    Entities(Vec<EntityId>),
}

#[cfg(test)]
pub fn hit_test_selection(snapshot: &LevelSnapshot, point: GridPoint) -> Selection {
    snapshot
        .entity_at(point)
        .map_or(Selection::Cell(point), |entity| {
            Selection::Entities(vec![entity.id().clone()])
        })
}

pub fn select_entity(
    current: Option<&Selection>,
    entity_id: EntityId,
    extend: bool,
) -> Option<Selection> {
    let Some(Selection::Entities(selected)) = current else {
        return Some(Selection::Entities(vec![entity_id]));
    };
    if !extend {
        return if selected.contains(&entity_id) {
            Some(Selection::Entities(selected.clone()))
        } else {
            Some(Selection::Entities(vec![entity_id]))
        };
    }

    let mut next = selected.clone();
    if let Some(index) = next.iter().position(|selected| selected == &entity_id) {
        next.remove(index);
    } else {
        next.push(entity_id);
    }
    (!next.is_empty()).then_some(Selection::Entities(next))
}

pub fn shape_from_designer_mask(mask: u64) -> Result<Shape, ShapeError> {
    let cells = (0..64_u8)
        .filter(|index| mask & (1_u64 << u32::from(*index)) != 0)
        .map(|index| ShapeCell::new(index % 8, index / 8))
        .collect::<Vec<_>>();
    let min_x = cells.iter().map(|cell| cell.x).min().unwrap_or_default();
    let min_y = cells.iter().map(|cell| cell.y).min().unwrap_or_default();
    let normalized = cells
        .into_iter()
        .map(|cell| ShapeCell::new(cell.x - min_x, cell.y - min_y))
        .collect::<Vec<_>>();
    Shape::from_cells(&normalized)
}

#[must_use]
pub fn designer_mask_from_shape(shape: Shape) -> u64 {
    shape
        .occupied_cells()
        .fold(0_u64, |mask, cell| mask | (1_u64 << (cell.x + cell.y * 8)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeBoundaryEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

#[must_use]
pub fn shape_boundary_edges(shape: Shape, cell: ShapeCell) -> ShapeBoundaryEdges {
    ShapeBoundaryEdges {
        left: cell.x == 0 || !shape.contains(ShapeCell::new(cell.x - 1, cell.y)),
        right: cell.x + 1 >= shape.width() || !shape.contains(ShapeCell::new(cell.x + 1, cell.y)),
        top: cell.y == 0 || !shape.contains(ShapeCell::new(cell.x, cell.y - 1)),
        bottom: cell.y + 1 >= shape.height() || !shape.contains(ShapeCell::new(cell.x, cell.y + 1)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoolTileGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PoolTileGeometry {
    #[must_use]
    pub fn pixel_at(self, x: f64, y: f64, resolution: u8) -> Option<(u8, u8)> {
        let local_x = x - self.x;
        let local_y = y - self.y;
        if resolution == 0
            || local_x < 0.0
            || local_y < 0.0
            || local_x >= self.width
            || local_y >= self.height
        {
            return None;
        }
        Some((
            (local_x * f64::from(resolution) / self.width).floor() as u8,
            (local_y * f64::from(resolution) / self.height).floor() as u8,
        ))
    }
}

#[must_use]
pub fn pool_tile_geometry(
    edges: ShapeBoundaryEdges,
    cell_size: f64,
    inset: f64,
) -> PoolTileGeometry {
    let x = if edges.left { inset } else { 0.0 };
    let y = if edges.top { inset } else { 0.0 };
    PoolTileGeometry {
        x,
        y,
        width: cell_size - x - if edges.right { inset } else { 0.0 },
        height: cell_size - y - if edges.bottom { inset } else { 0.0 },
    }
}

pub fn shape_world_points(origin: GridPoint, shape: Shape) -> Option<Vec<GridPoint>> {
    shape
        .occupied_cells()
        .map(|cell| {
            Some(GridPoint::new(
                origin.x.checked_add(u16::from(cell.x))?,
                origin.y.checked_add(u16::from(cell.y))?,
            ))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportTransform {
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

impl ViewportTransform {
    pub const MIN_SCALE: f64 = 0.08;
    pub const MAX_SCALE: f64 = 4.0;

    #[must_use]
    #[cfg(test)]
    pub fn frame(size: GridSize, canvas_size: f64, margin: f64, nominal_cell_size: f64) -> Self {
        Self::frame_rect(size, canvas_size, canvas_size, margin, nominal_cell_size)
    }

    #[must_use]
    pub fn frame_rect(
        size: GridSize,
        canvas_width: f64,
        canvas_height: f64,
        margin: f64,
        nominal_cell_size: f64,
    ) -> Self {
        let available_width = (canvas_width - margin * 2.0).max(nominal_cell_size);
        let available_height = (canvas_height - margin * 2.0).max(nominal_cell_size);
        let scale = (available_width / (f64::from(size.width()) * nominal_cell_size))
            .min(available_height / (f64::from(size.height()) * nominal_cell_size))
            .clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        let board_width = f64::from(size.width()) * nominal_cell_size * scale;
        let board_height = f64::from(size.height()) * nominal_cell_size * scale;
        Self {
            scale,
            offset_x: (canvas_width - board_width) / 2.0,
            offset_y: (canvas_height - board_height) / 2.0,
        }
    }

    #[must_use]
    pub fn point_at(
        self,
        x: f64,
        y: f64,
        size: GridSize,
        nominal_cell_size: f64,
    ) -> Option<GridPoint> {
        let cell_size = nominal_cell_size * self.scale;
        let board_x = x - self.offset_x;
        let board_y = y - self.offset_y;
        if board_x < 0.0
            || board_y < 0.0
            || board_x >= cell_size * f64::from(size.width())
            || board_y >= cell_size * f64::from(size.height())
        {
            return None;
        }
        Some(GridPoint::new(
            (board_x / cell_size).floor() as u16,
            (board_y / cell_size).floor() as u16,
        ))
    }

    #[must_use]
    pub fn cell_origin(self, point: GridPoint, nominal_cell_size: f64) -> (f64, f64) {
        let cell_size = nominal_cell_size * self.scale;
        (
            self.offset_x + f64::from(point.x) * cell_size,
            self.offset_y + f64::from(point.y) * cell_size,
        )
    }

    #[must_use]
    pub fn zoom_about(self, scale: f64, x: f64, y: f64) -> Self {
        let scale = scale.clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        let ratio = scale / self.scale;
        Self {
            scale,
            offset_x: x - (x - self.offset_x) * ratio,
            offset_y: y - (y - self.offset_y) * ratio,
        }
    }
}

pub fn blind_pixel_from_top_left_sample(
    entity: &PlaceableEntity,
    world_cell: GridPoint,
    pixel_x: u8,
    pixel_y_from_top: u8,
) -> Option<BlindPixel> {
    let blind = entity.as_blind()?;
    let resolution = blind.pixels_per_cell();
    if pixel_x >= resolution || pixel_y_from_top >= resolution {
        return None;
    }

    let cell_x = u8::try_from(world_cell.x.checked_sub(entity.origin().x)?).ok()?;
    let cell_y = u8::try_from(world_cell.y.checked_sub(entity.origin().y)?).ok()?;
    let cell = ShapeCell::new(cell_x, cell_y);
    blind.tile_for_cell(entity.shape(), cell)?;

    Some(BlindPixel::new(
        u16::from(cell_x) * u16::from(resolution) + u16::from(pixel_x),
        u16::from(cell_y) * u16::from(resolution) + u16::from(resolution - 1 - pixel_y_from_top),
    ))
}

pub fn rasterize_blind_segment(start: BlindPixel, end: BlindPixel) -> Vec<BlindPixel> {
    let mut pixels = Vec::new();
    let mut x = i32::from(start.x);
    let mut y = i32::from(start.y);
    let end_x = i32::from(end.x);
    let end_y = i32::from(end.y);
    let delta_x = (end_x - x).abs();
    let step_x = if x < end_x { 1 } else { -1 };
    let delta_y = -(end_y - y).abs();
    let step_y = if y < end_y { 1 } else { -1 };
    let mut error = delta_x + delta_y;

    loop {
        pixels.push(BlindPixel::new(x as u16, y as u16));
        if x == end_x && y == end_y {
            return pixels;
        }
        let doubled_error = error * 2;
        if doubled_error >= delta_y {
            error += delta_y;
            x += step_x;
        }
        if doubled_error <= delta_x {
            error += delta_x;
            y += step_y;
        }
    }
}

#[must_use]
pub fn entity_ids_in_rect(
    snapshot: &LevelSnapshot,
    start: GridPoint,
    end: GridPoint,
) -> Vec<EntityId> {
    let min_x = start.x.min(end.x);
    let max_x = start.x.max(end.x);
    let min_y = start.y.min(end.y);
    let max_y = start.y.max(end.y);
    snapshot
        .entities()
        .iter()
        .filter(|entity| {
            entity.shape().occupied_cells().any(|cell| {
                let x = entity.origin().x + u16::from(cell.x);
                let y = entity.origin().y + u16::from(cell.y);
                (min_x..=max_x).contains(&x) && (min_y..=max_y).contains(&y)
            })
        })
        .map(|entity| entity.id().clone())
        .collect()
}

pub fn key_locker_for_key<'a>(
    snapshot: &'a LevelSnapshot,
    entity_id: &EntityId,
) -> Option<&'a Decorator> {
    snapshot.decorators().iter().find(|decorator| {
        matches!(
            decorator.kind(),
            DecoratorKind::KeyLocker { key, .. } if key == entity_id
        )
    })
}

pub fn key_locker_for_lock<'a>(
    snapshot: &'a LevelSnapshot,
    entity_id: &EntityId,
) -> Option<&'a Decorator> {
    snapshot.decorators().iter().find(|decorator| {
        matches!(
            decorator.kind(),
            DecoratorKind::KeyLocker { entity, .. } if entity == entity_id
        )
    })
}

pub fn resolve_shortcut(
    key: &str,
    command_modifier: bool,
    shift: bool,
    scope: ShortcutScope,
) -> Option<Shortcut> {
    let key = key.to_ascii_lowercase();

    if command_modifier && key == "k" {
        return Some(Shortcut::TogglePalette);
    }
    if key == "escape" {
        return Some(Shortcut::CloseOverlay);
    }
    if scope == ShortcutScope::Palette && key == "enter" {
        return Some(Shortcut::PaletteSubmit);
    }
    if scope != ShortcutScope::Workspace {
        return None;
    }
    if command_modifier && !shift && key == "z" {
        return Some(Shortcut::Undo);
    }
    if command_modifier {
        return None;
    }

    match key.as_str() {
        "q" => Some(Shortcut::SelectMode(Mode::Select)),
        "w" => Some(Shortcut::SelectMode(Mode::Map)),
        "b" => Some(Shortcut::SelectMode(Mode::Brush)),
        "p" => Some(Shortcut::SelectMode(Mode::Sandbox)),
        "arrowup" => Some(Shortcut::MoveSelection(MoveDirection::Up)),
        "arrowright" => Some(Shortcut::MoveSelection(MoveDirection::Right)),
        "arrowdown" => Some(Shortcut::MoveSelection(MoveDirection::Down)),
        "arrowleft" => Some(Shortcut::MoveSelection(MoveDirection::Left)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelChange {
    Applied { sequence: u64 },
    NoChange,
}

#[derive(Debug)]
pub struct EditorModel {
    timeline: LevelTimeline,
    actor: ActorId,
    command_prefix: String,
    command_nonce: u64,
    entity_nonce: u64,
    decorator_nonce: u64,
}

#[derive(Debug)]
pub struct HydratedHistory {
    pub seen_commands: BTreeSet<String>,
    pub cell_blame: BTreeMap<GridPoint, BlameEntry>,
    pub entity_blame: BTreeMap<EntityId, BlameEntry>,
}

pub fn hydrate_history(
    expected_snapshot: &LevelSnapshot,
    events: &[HistoryEvent],
) -> Result<HydratedHistory, String> {
    let initial = LevelSnapshot::new(8, 8)
        .map_err(|error| format!("history base snapshot is invalid: {error}"))?;
    hydrate_history_from(initial, expected_snapshot, events)
}

pub fn hydrate_history_from(
    initial: LevelSnapshot,
    expected_snapshot: &LevelSnapshot,
    events: &[HistoryEvent],
) -> Result<HydratedHistory, String> {
    let mut timeline = LevelTimeline::new(initial)
        .map_err(|error| format!("history base timeline is invalid: {error}"))?;

    for (index, event) in events.iter().enumerate() {
        let expected_sequence = index as u64 + 1;
        if event.sequence != expected_sequence {
            return Err(format!(
                "history sequence {} appeared where {expected_sequence} was required",
                event.sequence
            ));
        }
        let reconstructed = if event.reverts_sequence.is_some() {
            timeline
                .undo_latest(event.metadata.clone())
                .map_err(|error| format!("history undo #{} is invalid: {error}", event.sequence))?
        } else {
            match timeline
                .apply(CommandEnvelope::new(
                    event.metadata.clone(),
                    event.command.clone(),
                ))
                .map_err(|error| format!("history event #{} is invalid: {error}", event.sequence))?
            {
                ApplyOutcome::Applied(reconstructed) => reconstructed,
                ApplyOutcome::NoChange { .. } => {
                    return Err(format!(
                        "history event #{} reconstructed as a no-op",
                        event.sequence
                    ));
                }
            }
        };
        if reconstructed != *event {
            return Err(format!(
                "history event #{} does not match deterministic core semantics",
                event.sequence
            ));
        }
    }

    if timeline.snapshot() != expected_snapshot {
        return Err("hydrated history does not produce the subscribed snapshot".to_owned());
    }

    let cell_targets: BTreeSet<_> = events
        .iter()
        .flat_map(|event| event.changes.iter())
        .filter_map(|change| match &change.target {
            LevelTarget::Cell(point) => Some(*point),
            _ => None,
        })
        .collect();
    let cell_blame = cell_targets
        .into_iter()
        .filter_map(|point| timeline.blame_cell(point).map(|blame| (point, blame)))
        .collect();
    let entity_targets: BTreeSet<_> = events
        .iter()
        .flat_map(|event| event.changes.iter())
        .filter_map(|change| match &change.target {
            LevelTarget::Entity(entity_id) => Some(entity_id.clone()),
            _ => None,
        })
        .collect();
    let entity_blame = entity_targets
        .into_iter()
        .filter_map(|entity_id| {
            timeline
                .blame_entity(&entity_id)
                .map(|blame| (entity_id, blame))
        })
        .collect();
    let seen_commands = events
        .iter()
        .map(|event| event.metadata.id.to_string())
        .collect();

    Ok(HydratedHistory {
        seen_commands,
        cell_blame,
        entity_blame,
    })
}

impl EditorModel {
    pub fn blank(actor_id: &str, command_session_prefix: &str) -> Self {
        Self::from_snapshot(
            LevelSnapshot::new(8, 8).expect("the fixed editor grid is valid"),
            actor_id,
            command_session_prefix,
        )
    }

    pub fn from_snapshot(
        snapshot: LevelSnapshot,
        actor_id: &str,
        command_session_prefix: &str,
    ) -> Self {
        Self {
            timeline: LevelTimeline::new(snapshot).expect("persisted snapshots are validated"),
            actor: ActorId::new(actor_id),
            command_prefix: format!("web-{command_session_prefix}"),
            command_nonce: 1,
            entity_nonce: 1,
            decorator_nonce: 1,
        }
    }

    pub const fn timeline(&self) -> &LevelTimeline {
        &self.timeline
    }

    pub const fn actor(&self) -> &ActorId {
        &self.actor
    }

    pub fn replace_snapshot(&mut self, snapshot: LevelSnapshot) -> Result<(), TimelineError> {
        self.timeline = LevelTimeline::new(snapshot)?;
        Ok(())
    }

    pub fn cell(&self, point: GridPoint) -> Result<CellKind, TimelineError> {
        self.timeline
            .snapshot()
            .cell(point)
            .map_err(TimelineError::from)
    }

    #[cfg(test)]
    pub fn toggle_cell(
        &mut self,
        point: GridPoint,
        occurred_at_ms: i64,
    ) -> Result<ModelChange, TimelineError> {
        let kind = match self.cell(point)? {
            CellKind::Floor => CellKind::Wall,
            CellKind::Wall => CellKind::Floor,
        };
        self.set_cell(point, kind, occurred_at_ms)
    }

    #[cfg(test)]
    pub fn set_cell(
        &mut self,
        point: GridPoint,
        kind: CellKind,
        occurred_at_ms: i64,
    ) -> Result<ModelChange, TimelineError> {
        let envelope = self.prepare_set_cell(point, kind, occurred_at_ms);
        self.apply_envelope(envelope)
    }

    pub fn prepare_set_cell(
        &mut self,
        point: GridPoint,
        kind: CellKind,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "edit",
            LevelCommand::SetCell { point, kind },
            occurred_at_ms,
        )
    }

    pub fn prepare_place_default_block(
        &mut self,
        point: GridPoint,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let shape = Shape::new(1, 1, 1).expect("the default Block shape is valid");
        self.prepare_place_block(point, shape, occurred_at_ms)
    }

    pub fn prepare_place_block(
        &mut self,
        point: GridPoint,
        shape: Shape,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let entity_id = self.next_entity_id("block");
        let block = Block::new(vec![CollectLayer::new(
            1,
            None,
            CollectCapacity::Unlimited,
            false,
        )]);
        let entity = PlaceableEntity::block(entity_id, point, shape, block)
            .expect("the default Block entity is valid");
        self.prepare_command(
            "place-block",
            LevelCommand::PlaceEntity { entity },
            occurred_at_ms,
        )
    }

    pub fn prepare_place_default_blind(
        &mut self,
        point: GridPoint,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let shape = Shape::new(1, 1, 1).expect("the default Blind shape is valid");
        self.prepare_place_blind(point, shape, occurred_at_ms)
    }

    pub fn prepare_place_blind(
        &mut self,
        point: GridPoint,
        shape: Shape,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let entity_id = self.next_entity_id("blind");
        let tiles = (0..shape.occupied_count())
            .map(|_| {
                BlindTile::empty(DEFAULT_BLIND_PIXELS_PER_CELL)
                    .expect("the default Blind tile is valid")
            })
            .collect();
        let blind = Blind::new(DEFAULT_BLIND_PIXELS_PER_CELL, tiles)
            .expect("the default Blind canvas is valid");
        let entity = PlaceableEntity::blind(entity_id, point, shape, blind)
            .expect("the default Blind entity is valid");
        self.prepare_command(
            "place-blind",
            LevelCommand::PlaceEntity { entity },
            occurred_at_ms,
        )
    }

    pub fn prepare_move_entities(
        &mut self,
        moves: Vec<EntityMove>,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "move-entities",
            LevelCommand::MoveEntities { moves },
            occurred_at_ms,
        )
    }

    pub fn prepare_resize_grid(
        &mut self,
        size: GridSize,
        anchor: GridAnchor,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "resize-grid",
            LevelCommand::ResizeGrid { size, anchor },
            occurred_at_ms,
        )
    }

    pub fn prepare_rotate_entity_clockwise(
        &mut self,
        entity_id: EntityId,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "rotate-entity",
            LevelCommand::RotateEntityClockwise { entity_id },
            occurred_at_ms,
        )
    }

    pub fn prepare_flip_entity_horizontal(
        &mut self,
        entity_id: EntityId,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "flip-entity",
            LevelCommand::FlipEntityHorizontal { entity_id },
            occurred_at_ms,
        )
    }

    pub fn prepare_set_blind_resolution(
        &mut self,
        entity_id: EntityId,
        pixels_per_cell: u8,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "set-blind-resolution",
            LevelCommand::SetBlindResolution {
                entity_id,
                pixels_per_cell,
            },
            occurred_at_ms,
        )
    }

    pub fn prepare_delete_entity(
        &mut self,
        entity_id: EntityId,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "delete-entity",
            LevelCommand::DeleteEntity { entity_id },
            occurred_at_ms,
        )
    }

    pub fn prepare_paint_blind_stroke(
        &mut self,
        entity_id: EntityId,
        color_index: u8,
        stroke: BlindStroke,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "paint-blind",
            LevelCommand::PaintBlindStroke {
                entity_id,
                color_index,
                stroke,
            },
            occurred_at_ms,
        )
    }

    pub fn prepare_erase_blind_stroke(
        &mut self,
        entity_id: EntityId,
        stroke: BlindStroke,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "erase-blind",
            LevelCommand::EraseBlindStroke { entity_id, stroke },
            occurred_at_ms,
        )
    }

    pub fn prepare_flood_fill_blind(
        &mut self,
        entity_id: EntityId,
        start: BlindPixel,
        color_index: u8,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        self.prepare_command(
            "fill-blind",
            LevelCommand::FloodFillBlind {
                entity_id,
                start,
                color_index,
            },
            occurred_at_ms,
        )
    }

    pub fn prepare_toggle_ice(
        &mut self,
        entity_id: EntityId,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let decorator_id = self.ordinary_decorator_id(&entity_id, "ice");
        self.prepare_command(
            "toggle-ice",
            LevelCommand::ToggleIce {
                decorator_id,
                entity_id,
            },
            occurred_at_ms,
        )
    }

    pub fn prepare_set_ice(
        &mut self,
        entity_id: EntityId,
        blocking_count: u32,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let decorator_id = self.ordinary_decorator_id(&entity_id, "ice");
        self.prepare_command(
            "set-ice",
            LevelCommand::SetIce {
                decorator_id,
                entity_id,
                blocking_count,
            },
            occurred_at_ms,
        )
    }

    pub fn prepare_cycle_direction(
        &mut self,
        entity_id: EntityId,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let decorator_id = self.ordinary_decorator_id(&entity_id, "direction");
        self.prepare_command(
            "cycle-direction",
            LevelCommand::CycleDirection {
                decorator_id,
                entity_id,
            },
            occurred_at_ms,
        )
    }

    pub fn prepare_set_direction(
        &mut self,
        entity_id: EntityId,
        mode: DirectionMode,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let decorator_id = self.ordinary_decorator_id(&entity_id, "direction");
        self.prepare_command(
            "set-direction",
            LevelCommand::SetDirection {
                decorator_id,
                entity_id,
                mode,
            },
            occurred_at_ms,
        )
    }

    pub fn prepare_assign_key_locker(
        &mut self,
        key_entity_id: EntityId,
        lock_entity_id: EntityId,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        let decorator_id = key_locker_for_key(self.timeline.snapshot(), &key_entity_id)
            .map(|decorator| decorator.id().clone())
            .unwrap_or_else(|| self.next_decorator_id("key-locker"));
        self.prepare_command(
            "assign-key-locker",
            LevelCommand::AssignKeyLocker {
                decorator_id,
                key_entity_id,
                lock_entity_id,
            },
            occurred_at_ms,
        )
    }

    pub fn apply_envelope(
        &mut self,
        envelope: CommandEnvelope,
    ) -> Result<ModelChange, TimelineError> {
        let outcome = self.timeline.apply(envelope)?;

        Ok(match outcome {
            ApplyOutcome::Applied(event) => ModelChange::Applied {
                sequence: event.sequence,
            },
            ApplyOutcome::NoChange { .. } => ModelChange::NoChange,
        })
    }

    #[cfg(test)]
    pub fn undo(&mut self, occurred_at_ms: i64) -> Result<HistoryEvent, TimelineError> {
        let metadata = self.prepare_undo(occurred_at_ms);
        self.timeline.undo_latest(metadata)
    }

    pub fn prepare_undo(&mut self, occurred_at_ms: i64) -> CommandMetadata {
        self.next_metadata("undo", occurred_at_ms)
    }

    pub fn apply_server_event(
        &mut self,
        event: &HistoryEvent,
    ) -> Result<ModelChange, TimelineError> {
        self.apply_envelope(CommandEnvelope::new(
            event.metadata.clone(),
            event.command.clone(),
        ))
    }

    fn next_metadata(&mut self, operation: &str, occurred_at_ms: i64) -> CommandMetadata {
        let id = format!(
            "{}-{operation}-{:016x}",
            self.command_prefix, self.command_nonce
        );
        self.command_nonce += 1;
        CommandMetadata::new(id, self.actor.clone(), occurred_at_ms)
    }

    fn prepare_command(
        &mut self,
        operation: &str,
        command: LevelCommand,
        occurred_at_ms: i64,
    ) -> CommandEnvelope {
        CommandEnvelope::new(self.next_metadata(operation, occurred_at_ms), command)
    }

    fn next_entity_id(&mut self, kind: &str) -> EntityId {
        loop {
            let id = EntityId::new(format!(
                "{}-{kind}-entity-{:016x}",
                self.command_prefix, self.entity_nonce
            ));
            self.entity_nonce += 1;
            if self.timeline.snapshot().entity(&id).is_none() {
                return id;
            }
        }
    }

    fn ordinary_decorator_id(&mut self, entity_id: &EntityId, kind: &str) -> DecoratorId {
        let existing = match kind {
            "ice" => self.timeline.snapshot().ice_for_entity(entity_id),
            "direction" => self.timeline.snapshot().direction_for_entity(entity_id),
            _ => None,
        };
        existing
            .map(|decorator| decorator.id().clone())
            .unwrap_or_else(|| self.next_decorator_id(kind))
    }

    fn next_decorator_id(&mut self, kind: &str) -> DecoratorId {
        loop {
            let id = DecoratorId::new(format!(
                "{}-{kind}-decorator-{:016x}",
                self.command_prefix, self.decorator_nonce
            ));
            self.decorator_nonce += 1;
            let collides_with_entity = self
                .timeline
                .snapshot()
                .entities()
                .iter()
                .any(|entity| entity.id().as_str() == id.as_str());
            if !collides_with_entity && self.timeline.snapshot().decorator(&id).is_none() {
                return id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_roster_tracks_tabs_but_counts_distinct_actors() {
        let target = ProjectLevelTarget::new("project", "level");
        let self_id = PresenceId::new("self-tab");
        let other_id = PresenceId::new("other-tab");
        let actor = ActorId::new("alice");
        let mut roster = PresenceRoster::default();

        assert!(roster.apply(
            &target,
            LevelPresenceItem::Snapshot {
                target: target.clone(),
                self_id: self_id.clone(),
                participants: vec![PresenceParticipant {
                    id: self_id.clone(),
                    actor: actor.clone(),
                    cursor: None,
                }],
            },
        ));
        assert!(roster.apply(
            &target,
            LevelPresenceItem::Joined {
                target: target.clone(),
                participant: PresenceParticipant {
                    id: other_id.clone(),
                    actor,
                    cursor: None,
                },
            },
        ));
        assert_eq!(roster.participant_count(), 2);
        assert_eq!(roster.actor_count(), 1);
        assert!(roster.is_self(&self_id));

        assert!(roster.apply(
            &target,
            LevelPresenceItem::Cursor {
                target: target.clone(),
                presence_id: other_id.clone(),
                cursor: Some(GridPoint::new(3, 4)),
            },
        ));
        assert_eq!(
            roster.participant(&other_id).and_then(|entry| entry.cursor),
            Some(GridPoint::new(3, 4))
        );
        assert!(roster.apply(
            &target,
            LevelPresenceItem::Left {
                target: target.clone(),
                presence_id: other_id,
            },
        ));
        assert_eq!(roster.participant_count(), 1);
        assert_eq!(roster.participants().count(), 1);
        roster.clear();
        assert_eq!(roster.participant_count(), 0);
    }

    #[test]
    fn workspace_shortcuts_select_all_modes_and_undo() {
        let scope = ShortcutScope::Workspace;
        assert_eq!(
            resolve_shortcut("q", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Select))
        );
        assert_eq!(
            resolve_shortcut("W", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Map))
        );
        assert_eq!(
            resolve_shortcut("b", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Brush))
        );
        assert_eq!(
            resolve_shortcut("p", false, false, scope),
            Some(Shortcut::SelectMode(Mode::Sandbox))
        );
        assert_eq!(
            resolve_shortcut("z", true, false, scope),
            Some(Shortcut::Undo)
        );
        assert_eq!(
            resolve_shortcut("ArrowLeft", false, false, scope),
            Some(Shortcut::MoveSelection(MoveDirection::Left))
        );
        assert_eq!(MoveDirection::Up.offset(), (0, -1));
        assert_eq!(MoveDirection::Right.offset(), (1, 0));
    }

    #[test]
    fn text_entry_scope_does_not_steal_editor_keys() {
        let scope = ShortcutScope::TextEntry;
        assert_eq!(resolve_shortcut("q", false, false, scope), None);
        assert_eq!(resolve_shortcut("z", true, false, scope), None);
        assert_eq!(
            resolve_shortcut("k", true, false, scope),
            Some(Shortcut::TogglePalette)
        );
        assert_eq!(
            resolve_shortcut("Escape", false, false, scope),
            Some(Shortcut::CloseOverlay)
        );
    }

    #[test]
    fn palette_scope_submits_without_enabling_workspace_shortcuts() {
        let scope = ShortcutScope::Palette;
        assert_eq!(
            resolve_shortcut("Enter", false, false, scope),
            Some(Shortcut::PaletteSubmit)
        );
        assert_eq!(resolve_shortcut("b", false, false, scope), None);
    }

    #[test]
    fn reducer_uses_core_for_toggle_blame_and_user_scoped_undo() {
        let point = GridPoint::new(3, 4);
        let mut model = EditorModel::blank("user-123", "test-session");

        assert_eq!(
            model.toggle_cell(point, 1_000).unwrap(),
            ModelChange::Applied { sequence: 1 }
        );
        assert_eq!(model.cell(point).unwrap(), CellKind::Wall);
        let blame = model.timeline().blame_cell(point).unwrap();
        assert_eq!(blame.actor.as_str(), "user-123");
        assert_eq!(blame.occurred_at_ms, 1_000);

        let undo = model.undo(2_000).unwrap();
        assert_eq!(undo.sequence, 2);
        assert_eq!(undo.reverts_sequence, Some(1));
        assert_eq!(model.cell(point).unwrap(), CellKind::Floor);
    }

    #[test]
    fn command_ids_are_session_unique_and_survive_snapshot_replacement() {
        let point = GridPoint::new(0, 0);
        let mut first = EditorModel::blank("same-user", "session-a");
        let mut second = EditorModel::blank("same-user", "session-b");

        let first_command = first.prepare_set_cell(point, CellKind::Wall, 100);
        let second_command = second.prepare_set_cell(point, CellKind::Wall, 100);
        assert_eq!(first.actor().as_str(), "same-user");
        assert_ne!(first_command.metadata.id, second_command.metadata.id);
        assert_eq!(first_command.metadata.actor, second_command.metadata.actor);

        first
            .replace_snapshot(LevelSnapshot::new(8, 8).unwrap())
            .unwrap();
        let after_resync = first.prepare_set_cell(point, CellKind::Wall, 101);
        assert_ne!(first_command.metadata.id, after_resync.metadata.id);
        assert_eq!(first_command.metadata.actor, after_resync.metadata.actor);
    }

    #[test]
    fn authoritative_event_is_revalidated_by_the_shared_core() {
        let point = GridPoint::new(6, 2);
        let mut source = EditorModel::blank("remote-user", "remote-session");
        source.set_cell(point, CellKind::Wall, 500).unwrap();
        let event = source.timeline().events()[0].clone();

        let mut replica = EditorModel::blank("local-user", "local-session");
        assert_eq!(
            replica.apply_server_event(&event).unwrap(),
            ModelChange::Applied { sequence: 1 }
        );
        assert_eq!(replica.cell(point).unwrap(), CellKind::Wall);
    }

    #[test]
    fn hydrated_history_reconstructs_reverted_cell_provenance() {
        let reverted = GridPoint::new(1, 1);
        let active = GridPoint::new(2, 2);
        let mut source = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
        source
            .apply(CommandEnvelope::new(
                CommandMetadata::new("alice-wall", "alice", 100),
                LevelCommand::SetCell {
                    point: reverted,
                    kind: CellKind::Wall,
                },
            ))
            .unwrap();
        source
            .apply(CommandEnvelope::new(
                CommandMetadata::new("bob-wall", "bob", 110),
                LevelCommand::SetCell {
                    point: active,
                    kind: CellKind::Wall,
                },
            ))
            .unwrap();
        source
            .undo_latest(CommandMetadata::new("alice-undo", "alice", 120))
            .unwrap();

        let hydration = hydrate_history(source.snapshot(), source.events()).unwrap();
        let reverted_blame = hydration.cell_blame.get(&reverted).unwrap();
        assert_eq!(reverted_blame.sequence, 3);
        assert_eq!(reverted_blame.command_id.as_str(), "alice-undo");
        assert_eq!(reverted_blame.reverts_sequence, Some(1));
        let active_blame = hydration.cell_blame.get(&active).unwrap();
        assert_eq!(active_blame.sequence, 2);
        assert_eq!(active_blame.actor.as_str(), "bob");
        assert_eq!(hydration.seen_commands.len(), 3);
    }

    #[test]
    fn hit_testing_prefers_an_entity_footprint_over_its_floor_cell() {
        let entity = PlaceableEntity::block(
            "block",
            GridPoint::new(2, 3),
            Shape::new(2, 1, 0b11).unwrap(),
            Block::default(),
        )
        .unwrap();
        let size = oreak_core::GridSize::new(8, 8).unwrap();
        let snapshot = LevelSnapshot::from_parts(
            size,
            vec![CellKind::Floor; size.cell_count()],
            vec![entity],
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            hit_test_selection(&snapshot, GridPoint::new(3, 3)),
            Selection::Entities(vec![EntityId::from("block")])
        );
        assert_eq!(
            hit_test_selection(&snapshot, GridPoint::new(4, 3)),
            Selection::Cell(GridPoint::new(4, 3))
        );
    }

    #[test]
    fn extended_entity_selection_preserves_click_order() {
        let a = EntityId::from("a");
        let b = EntityId::from("b");
        let c = EntityId::from("c");
        let selection = select_entity(None, a.clone(), false);
        let selection = select_entity(selection.as_ref(), b.clone(), true);
        let selection = select_entity(selection.as_ref(), c.clone(), true);
        assert_eq!(
            selection,
            Some(Selection::Entities(vec![a.clone(), b.clone(), c]))
        );

        let selection = select_entity(selection.as_ref(), b, true);
        assert_eq!(
            selection,
            Some(Selection::Entities(vec![a, EntityId::from("c")]))
        );
    }

    #[test]
    fn plain_click_on_selected_group_member_preserves_the_group() {
        let selection = Selection::Entities(vec![EntityId::from("a"), EntityId::from("b")]);
        assert_eq!(
            select_entity(Some(&selection), EntityId::from("a"), false),
            Some(selection)
        );
    }

    #[test]
    fn viewport_frame_and_hit_testing_support_non_square_grids() {
        let size = GridSize::new(16, 4).unwrap();
        let viewport = ViewportTransform::frame(size, 768.0, 44.0, 85.0);
        let (left, top) = viewport.cell_origin(GridPoint::new(0, 0), 85.0);
        let cell_size = 85.0 * viewport.scale;
        assert!((left - 44.0).abs() < 0.001);
        assert!(top > 250.0);
        assert_eq!(
            viewport.point_at(left + cell_size / 2.0, top + cell_size / 2.0, size, 85.0),
            Some(GridPoint::new(0, 0))
        );
        assert_eq!(
            viewport.point_at(left + cell_size * 16.0, top + cell_size, size, 85.0),
            None
        );
        let zoomed = viewport.zoom_about(viewport.scale * 2.0, left, top);
        assert!((zoomed.offset_x - left).abs() < 0.001);
        assert!((zoomed.offset_y - top).abs() < 0.001);
    }

    #[test]
    fn rectangular_viewport_frames_without_stretching_cells() {
        let size = GridSize::new(8, 8).unwrap();
        let viewport = ViewportTransform::frame_rect(size, 1200.0, 600.0, 24.0, 85.0);
        let board_width = f64::from(size.width()) * 85.0 * viewport.scale;
        let board_height = f64::from(size.height()) * 85.0 * viewport.scale;

        assert!((board_width - board_height).abs() < 0.001);
        assert!((viewport.offset_x - (1200.0 - board_width) / 2.0).abs() < 0.001);
        assert!((viewport.offset_y - (600.0 - board_height) / 2.0).abs() < 0.001);
    }

    #[test]
    fn edge_resize_plans_keep_the_opposite_visual_edge_fixed() {
        let size = GridSize::new(8, 6).unwrap();
        assert_eq!(
            plan_edge_resize(size, MapResizeEdge::Top, 2),
            (GridSize::new(8, 8).unwrap(), GridAnchor::Top)
        );
        assert_eq!(
            plan_edge_resize(size, MapResizeEdge::Right, 3),
            (GridSize::new(11, 6).unwrap(), GridAnchor::Left)
        );
        assert_eq!(
            plan_edge_resize(size, MapResizeEdge::Bottom, -2),
            (GridSize::new(8, 4).unwrap(), GridAnchor::Bottom)
        );
        assert_eq!(
            plan_edge_resize(size, MapResizeEdge::Left, 1),
            (GridSize::new(9, 6).unwrap(), GridAnchor::Right)
        );
    }

    #[test]
    fn edge_resize_plans_clamp_to_core_grid_limits() {
        let size = GridSize::new(8, 6).unwrap();
        assert_eq!(
            plan_edge_resize(size, MapResizeEdge::Left, -100).0.width(),
            1
        );
        assert_eq!(
            plan_edge_resize(size, MapResizeEdge::Top, 1_000).0.height(),
            256
        );
    }

    #[test]
    fn content_aware_edge_resize_does_not_clip_authored_cells() {
        let mut model = EditorModel::blank("actor", "session");
        assert!(matches!(
            model.set_cell(GridPoint::new(2, 1), CellKind::Wall, 10),
            Ok(ModelChange::Applied { .. })
        ));
        let snapshot = model.timeline().snapshot();

        assert_eq!(
            plan_content_aware_edge_resize(snapshot, MapResizeEdge::Top, -100)
                .0
                .height(),
            7
        );
        assert_eq!(
            plan_content_aware_edge_resize(snapshot, MapResizeEdge::Right, -100)
                .0
                .width(),
            3
        );
        assert_eq!(
            plan_content_aware_edge_resize(snapshot, MapResizeEdge::Bottom, -100)
                .0
                .height(),
            2
        );
        assert_eq!(
            plan_content_aware_edge_resize(snapshot, MapResizeEdge::Left, -100)
                .0
                .width(),
            6
        );
    }

    #[test]
    fn content_aware_edge_resize_allows_empty_floor_to_reach_minimum_size() {
        let snapshot = LevelSnapshot::new(8, 8).unwrap();
        for edge in [
            MapResizeEdge::Top,
            MapResizeEdge::Right,
            MapResizeEdge::Bottom,
            MapResizeEdge::Left,
        ] {
            let size = plan_content_aware_edge_resize(&snapshot, edge, -100).0;
            let resized_axis = match edge {
                MapResizeEdge::Top | MapResizeEdge::Bottom => size.height(),
                MapResizeEdge::Right | MapResizeEdge::Left => size.width(),
            };
            assert_eq!(resized_axis, 1);
        }
    }

    #[test]
    fn reconnect_backoff_is_bounded() {
        assert_eq!(reconnect_delay_ms(1), 1_000);
        assert_eq!(reconnect_delay_ms(2), 2_000);
        assert_eq!(reconnect_delay_ms(3), 4_000);
        assert_eq!(reconnect_delay_ms(4), 8_000);
        assert_eq!(reconnect_delay_ms(5), 15_000);
        assert_eq!(reconnect_delay_ms(20), 15_000);
    }

    #[test]
    fn designer_masks_are_normalized_and_validated_by_core_shape_rules() {
        let offset_corner =
            (1_u64 << (2 + 3 * 8)) | (1_u64 << (3 + 3 * 8)) | (1_u64 << (2 + 4 * 8));
        let shape = shape_from_designer_mask(offset_corner).unwrap();
        assert_eq!((shape.width(), shape.height()), (2, 2));
        assert_eq!(shape.occupied_mask(), 0b0111);
        assert_eq!(
            designer_mask_from_shape(shape),
            (1 << 0) | (1 << 1) | (1 << 8)
        );

        let disconnected = (1_u64 << 0) | (1_u64 << 2);
        assert!(shape_from_designer_mask(disconnected).is_err());
        assert!(shape_from_designer_mask(0).is_err());
    }

    #[test]
    fn connected_shape_edges_only_expose_the_outer_boundary() {
        let shape = Shape::from_cells(&[
            ShapeCell::new(0, 0),
            ShapeCell::new(1, 0),
            ShapeCell::new(0, 1),
        ])
        .expect("connected L shape");

        assert_eq!(
            shape_boundary_edges(shape, ShapeCell::new(0, 0)),
            ShapeBoundaryEdges {
                left: true,
                right: false,
                top: true,
                bottom: false,
            }
        );
        assert_eq!(
            shape_boundary_edges(shape, ShapeCell::new(1, 0)),
            ShapeBoundaryEdges {
                left: false,
                right: true,
                top: true,
                bottom: true,
            }
        );
    }

    #[test]
    fn pool_tile_geometry_keeps_internal_seams_paintable() {
        let left = pool_tile_geometry(
            ShapeBoundaryEdges {
                left: true,
                right: false,
                top: true,
                bottom: true,
            },
            85.0,
            6.0,
        );
        let right = pool_tile_geometry(
            ShapeBoundaryEdges {
                left: false,
                right: true,
                top: true,
                bottom: true,
            },
            85.0,
            6.0,
        );

        assert_eq!(
            left,
            PoolTileGeometry {
                x: 6.0,
                y: 6.0,
                width: 79.0,
                height: 73.0
            }
        );
        assert_eq!(
            right,
            PoolTileGeometry {
                x: 0.0,
                y: 6.0,
                width: 79.0,
                height: 73.0
            }
        );
        assert_eq!(left.pixel_at(84.9, 42.0, 4), Some((3, 1)));
        assert_eq!(right.pixel_at(0.0, 42.0, 4), Some((0, 1)));
    }

    #[test]
    fn template_placement_keeps_connected_cells_in_one_entity() {
        let shape = Shape::from_cells(&[
            ShapeCell::new(0, 0),
            ShapeCell::new(1, 0),
            ShapeCell::new(0, 1),
        ])
        .expect("connected L shape");
        let mut model = EditorModel::blank("alice", "solid-shape-session");

        for envelope in [
            model.prepare_place_block(GridPoint::new(1, 1), shape, 10),
            model.prepare_place_blind(GridPoint::new(4, 1), shape, 11),
        ] {
            let LevelCommand::PlaceEntity { entity } = envelope.command else {
                panic!("expected one entity placement");
            };
            assert_eq!(entity.shape(), shape);
            if let Some(pool) = entity.as_blind() {
                assert_eq!(pool.tiles().len(), shape.occupied_count() as usize);
            }
        }
    }

    #[test]
    fn marquee_rectangle_selects_intersecting_entities() {
        let mut model = EditorModel::blank("alice", "marquee-session");
        let first = model.prepare_place_default_block(GridPoint::new(1, 1), 100);
        let first_id = match &first.command {
            LevelCommand::PlaceEntity { entity } => entity.id().clone(),
            _ => unreachable!(),
        };
        model.apply_envelope(first).unwrap();
        let second = model.prepare_place_default_blind(GridPoint::new(5, 5), 101);
        model.apply_envelope(second).unwrap();

        assert_eq!(
            entity_ids_in_rect(
                model.timeline().snapshot(),
                GridPoint::new(0, 0),
                GridPoint::new(2, 2),
            ),
            vec![first_id]
        );
    }

    #[test]
    fn connected_shape_world_points_cover_the_complete_pending_footprint() {
        let shape = Shape::from_cells(&[
            ShapeCell::new(0, 0),
            ShapeCell::new(1, 0),
            ShapeCell::new(0, 1),
        ])
        .unwrap();

        assert_eq!(
            shape_world_points(GridPoint::new(5, 7), shape).unwrap(),
            vec![
                GridPoint::new(5, 7),
                GridPoint::new(6, 7),
                GridPoint::new(5, 8),
            ]
        );
    }

    #[test]
    fn resized_history_hydrates_from_the_level_genesis() {
        let initial = LevelSnapshot::new(8, 8).unwrap();
        let mut model = EditorModel::blank("alice", "resize-session");
        assert!(matches!(
            model
                .prepare_resize_grid(GridSize::new(12, 6).unwrap(), GridAnchor::Center, 99)
                .command,
            LevelCommand::ResizeGrid { .. }
        ));
        let mut timeline = LevelTimeline::new(initial.clone()).unwrap();
        timeline
            .apply(CommandEnvelope::new(
                CommandMetadata::new("resize", "alice", 100),
                LevelCommand::ResizeGrid {
                    size: GridSize::new(12, 6).unwrap(),
                    anchor: GridAnchor::Center,
                },
            ))
            .unwrap();

        let hydration =
            hydrate_history_from(initial, timeline.snapshot(), timeline.events()).unwrap();
        assert!(hydration.seen_commands.contains("resize"));
    }

    #[test]
    fn entity_command_preparation_uses_session_ids_and_valid_defaults() {
        let point = GridPoint::new(3, 4);
        let mut model = EditorModel::blank("alice", "entity-session");
        let block_envelope = model.prepare_place_default_block(point, 100);
        let blind_envelope = model.prepare_place_default_blind(GridPoint::new(5, 4), 101);

        let LevelCommand::PlaceEntity { entity: block } = &block_envelope.command else {
            panic!("expected a Block placement");
        };
        let oreak_core::PlaceableEntityKind::Block(block_kind) = block.kind() else {
            panic!("expected a Block");
        };
        assert_eq!(block.origin(), point);
        assert!(
            block
                .id()
                .as_str()
                .contains("web-entity-session-block-entity")
        );
        assert_eq!(block_kind.collect_layers().len(), 1);
        assert_eq!(block_kind.collect_layers()[0].color_index(), 1);
        assert_eq!(
            block_kind.collect_layers()[0].capacity(),
            CollectCapacity::Unlimited
        );
        let block_id = block.id().clone();

        let LevelCommand::PlaceEntity { entity: blind } = &blind_envelope.command else {
            panic!("expected a Blind placement");
        };
        let oreak_core::PlaceableEntityKind::Blind(blind_kind) = blind.kind() else {
            panic!("expected a Blind");
        };
        assert_ne!(block.id(), blind.id());
        assert_eq!(blind_kind.pixels_per_cell(), DEFAULT_BLIND_PIXELS_PER_CELL);
        assert_eq!(blind_kind.tiles().len(), 1);
        assert!(
            blind_kind.tiles()[0]
                .colors()
                .iter()
                .all(|color| *color == 0)
        );

        let blind_command_id = blind_envelope.metadata.id.clone();
        model.apply_envelope(block_envelope).unwrap();
        let moved = model.prepare_move_entities(
            vec![EntityMove::new(block_id.clone(), GridPoint::new(4, 4))],
            102,
        );
        assert!(matches!(
            moved.command,
            LevelCommand::MoveEntities { moves }
                if moves == vec![EntityMove::new(block_id.clone(), GridPoint::new(4, 4))]
        ));
        assert_ne!(moved.metadata.id, blind_command_id);
        assert!(matches!(
            model
                .prepare_rotate_entity_clockwise(block_id.clone(), 103)
                .command,
            LevelCommand::RotateEntityClockwise { .. }
        ));
        assert!(matches!(
            model
                .prepare_flip_entity_horizontal(block_id.clone(), 104)
                .command,
            LevelCommand::FlipEntityHorizontal { .. }
        ));
        assert!(matches!(
            model
                .prepare_set_blind_resolution(blind.id().clone(), 8, 104)
                .command,
            LevelCommand::SetBlindResolution {
                pixels_per_cell: 8,
                ..
            }
        ));
        assert!(matches!(
            model.prepare_delete_entity(block_id, 105).command,
            LevelCommand::DeleteEntity { .. }
        ));
    }

    #[test]
    fn blind_commands_prepare_and_apply_through_the_shared_core() {
        let mut model = EditorModel::blank("alice", "brush-session");
        let placement = model.prepare_place_default_blind(GridPoint::new(2, 2), 100);
        let LevelCommand::PlaceEntity { entity } = &placement.command else {
            panic!("expected a Blind placement");
        };
        let entity_id = entity.id().clone();
        model.apply_envelope(placement).unwrap();

        let paint = model.prepare_paint_blind_stroke(
            entity_id.clone(),
            4,
            BlindStroke::new(vec![BlindPixel::new(0, 0), BlindPixel::new(1, 0)]).unwrap(),
            101,
        );
        assert!(paint.metadata.id.as_str().contains("paint-blind"));
        assert!(matches!(
            &paint.command,
            LevelCommand::PaintBlindStroke {
                entity_id: target,
                color_index: 4,
                stroke,
            } if target == &entity_id && stroke.pixels().len() == 2
        ));
        model.apply_envelope(paint).unwrap();

        let erase = model.prepare_erase_blind_stroke(
            entity_id.clone(),
            BlindStroke::new(vec![BlindPixel::new(1, 0)]).unwrap(),
            102,
        );
        assert!(erase.metadata.id.as_str().contains("erase-blind"));
        model.apply_envelope(erase).unwrap();

        let fill = model.prepare_flood_fill_blind(entity_id.clone(), BlindPixel::new(1, 0), 7, 103);
        assert!(fill.metadata.id.as_str().contains("fill-blind"));
        model.apply_envelope(fill).unwrap();

        let entity = model.timeline().snapshot().entity(&entity_id).unwrap();
        let blind = entity.as_blind().unwrap();
        assert_eq!(
            blind.color_at(entity.shape(), BlindPixel::new(0, 0)),
            Some(4)
        );
        assert_eq!(
            blind.color_at(entity.shape(), BlindPixel::new(1, 0)),
            Some(7)
        );
        assert_eq!(model.timeline().events().len(), 4);
    }

    #[test]
    fn top_left_samples_map_to_lower_left_blind_pixels() {
        let shape = Shape::new(2, 2, 0b1111).unwrap();
        let tiles = (0..4).map(|_| BlindTile::empty(4).unwrap()).collect();
        let entity = PlaceableEntity::blind(
            "blind",
            GridPoint::new(2, 3),
            shape,
            Blind::new(4, tiles).unwrap(),
        )
        .unwrap();

        assert_eq!(
            blind_pixel_from_top_left_sample(&entity, GridPoint::new(2, 3), 0, 0),
            Some(BlindPixel::new(0, 3))
        );
        assert_eq!(
            blind_pixel_from_top_left_sample(&entity, GridPoint::new(2, 3), 3, 3),
            Some(BlindPixel::new(3, 0))
        );
        assert_eq!(
            blind_pixel_from_top_left_sample(&entity, GridPoint::new(3, 4), 1, 2),
            Some(BlindPixel::new(5, 5))
        );
        assert_eq!(
            blind_pixel_from_top_left_sample(&entity, GridPoint::new(1, 3), 0, 0),
            None
        );
        assert_eq!(
            blind_pixel_from_top_left_sample(&entity, GridPoint::new(2, 3), 4, 0),
            None
        );

        let shape_with_hole = Shape::new(3, 3, 0b111_101_111).unwrap();
        let tiles = (0..8).map(|_| BlindTile::empty(1).unwrap()).collect();
        let ring = PlaceableEntity::blind(
            "ring",
            GridPoint::new(0, 0),
            shape_with_hole,
            Blind::new(1, tiles).unwrap(),
        )
        .unwrap();
        assert_eq!(
            blind_pixel_from_top_left_sample(&ring, GridPoint::new(1, 1), 0, 0),
            None
        );
    }

    #[test]
    fn blind_segment_rasterization_is_continuous_and_includes_endpoints() {
        assert_eq!(
            rasterize_blind_segment(BlindPixel::new(1, 1), BlindPixel::new(4, 3)),
            vec![
                BlindPixel::new(1, 1),
                BlindPixel::new(2, 2),
                BlindPixel::new(3, 2),
                BlindPixel::new(4, 3),
            ]
        );
        assert_eq!(
            rasterize_blind_segment(BlindPixel::new(4, 3), BlindPixel::new(1, 1)),
            vec![
                BlindPixel::new(4, 3),
                BlindPixel::new(3, 2),
                BlindPixel::new(2, 2),
                BlindPixel::new(1, 1),
            ]
        );
        assert_eq!(
            rasterize_blind_segment(BlindPixel::new(2, 2), BlindPixel::new(2, 2)),
            vec![BlindPixel::new(2, 2)]
        );
    }

    #[test]
    fn decorator_commands_reuse_ids_and_preserve_key_locker_orientation() {
        let mut model = EditorModel::blank("alice", "decorator-session");
        let mut block_ids = Vec::new();
        for (x, time) in [(0, 100), (2, 101), (4, 102)] {
            let placement = model.prepare_place_default_block(GridPoint::new(x, 0), time);
            let LevelCommand::PlaceEntity { entity } = &placement.command else {
                panic!("expected Block placement");
            };
            block_ids.push(entity.id().clone());
            model.apply_envelope(placement).unwrap();
        }

        let toggle = model.prepare_toggle_ice(block_ids[0].clone(), 110);
        let LevelCommand::ToggleIce {
            decorator_id: ice_id,
            ..
        } = &toggle.command
        else {
            panic!("expected ToggleIce");
        };
        let ice_id = ice_id.clone();
        model.apply_envelope(toggle).unwrap();
        let set = model.prepare_set_ice(block_ids[0].clone(), 5, 111);
        assert!(matches!(
            &set.command,
            LevelCommand::SetIce {
                decorator_id,
                blocking_count: 5,
                ..
            } if decorator_id == &ice_id
        ));
        model.apply_envelope(set).unwrap();

        let direction = model.prepare_cycle_direction(block_ids[0].clone(), 112);
        let LevelCommand::CycleDirection {
            decorator_id: direction_id,
            ..
        } = &direction.command
        else {
            panic!("expected CycleDirection");
        };
        let direction_id = direction_id.clone();
        model.apply_envelope(direction).unwrap();
        assert!(matches!(
            model
                .prepare_set_direction(block_ids[0].clone(), DirectionMode::Vertical, 113)
                .command,
            LevelCommand::SetDirection { decorator_id, .. } if decorator_id == direction_id
        ));

        let relation =
            model.prepare_assign_key_locker(block_ids[0].clone(), block_ids[1].clone(), 114);
        let LevelCommand::AssignKeyLocker {
            decorator_id: relation_id,
            ..
        } = &relation.command
        else {
            panic!("expected AssignKeyLocker");
        };
        let relation_id = relation_id.clone();
        model.apply_envelope(relation).unwrap();
        let snapshot = model.timeline().snapshot();
        assert_eq!(
            key_locker_for_key(snapshot, &block_ids[0]).map(Decorator::id),
            Some(&relation_id)
        );
        assert_eq!(
            key_locker_for_lock(snapshot, &block_ids[1]).map(Decorator::id),
            Some(&relation_id)
        );

        let reassign =
            model.prepare_assign_key_locker(block_ids[0].clone(), block_ids[2].clone(), 115);
        assert!(matches!(
            &reassign.command,
            LevelCommand::AssignKeyLocker { decorator_id, lock_entity_id, .. }
                if decorator_id == &relation_id && lock_entity_id == &block_ids[2]
        ));
    }

    #[test]
    fn generated_entity_ids_skip_existing_session_ids() {
        let existing = PlaceableEntity::block(
            "web-collision-session-block-entity-0000000000000001",
            GridPoint::new(0, 0),
            Shape::new(1, 1, 1).unwrap(),
            Block::default(),
        )
        .unwrap();
        let size = oreak_core::GridSize::new(8, 8).unwrap();
        let snapshot = LevelSnapshot::from_parts(
            size,
            vec![CellKind::Floor; size.cell_count()],
            vec![existing],
            Vec::new(),
        )
        .unwrap();
        let mut model = EditorModel::from_snapshot(snapshot, "alice", "collision-session");

        let command = model.prepare_place_default_block(GridPoint::new(1, 0), 100);
        let LevelCommand::PlaceEntity { entity } = command.command else {
            panic!("expected a Block placement");
        };
        assert_eq!(
            entity.id().as_str(),
            "web-collision-session-block-entity-0000000000000002"
        );
    }

    #[test]
    fn hydrated_history_includes_entity_blame_without_changing_cell_blame() {
        let mut source = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
        let entity = PlaceableEntity::block(
            "entity",
            GridPoint::new(1, 1),
            Shape::new(1, 1, 1).unwrap(),
            Block::default(),
        )
        .unwrap();
        source
            .apply(CommandEnvelope::new(
                CommandMetadata::new("place", "alice", 100),
                LevelCommand::PlaceEntity { entity },
            ))
            .unwrap();
        source
            .apply(CommandEnvelope::new(
                CommandMetadata::new("wall", "bob", 110),
                LevelCommand::SetCell {
                    point: GridPoint::new(0, 0),
                    kind: CellKind::Wall,
                },
            ))
            .unwrap();

        let hydration = hydrate_history(source.snapshot(), source.events()).unwrap();
        assert_eq!(
            hydration.entity_blame[&EntityId::from("entity")]
                .command_id
                .as_str(),
            "place"
        );
        assert_eq!(
            hydration.cell_blame[&GridPoint::new(0, 0)]
                .command_id
                .as_str(),
            "wall"
        );
    }
}
