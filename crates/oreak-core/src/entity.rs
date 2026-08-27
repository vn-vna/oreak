use std::collections::{BTreeSet, VecDeque};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::brush::validate_color_index;
use crate::{
    BlindBrushOperation, BlindGuide, BlindGuidePatch, BlindPixel, BlindTilePatch, BrushError,
    EntityId, GridPoint, MAX_BLIND_GUIDES, MAX_BLOCK_SHAPE_AXIS, MAX_SHAPE_AREA, Shape, ShapeCell,
};

pub const MIN_BLIND_PIXELS_PER_CELL: u8 = 1;
pub const MAX_BLIND_PIXELS_PER_CELL: u8 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectCapacity {
    Unlimited,
    Finite(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectLayer {
    color_index: u16,
    radius: Option<u8>,
    capacity: CollectCapacity,
    locked: bool,
}

impl CollectLayer {
    #[must_use]
    pub const fn new(
        color_index: u16,
        radius: Option<u8>,
        capacity: CollectCapacity,
        locked: bool,
    ) -> Self {
        Self {
            color_index,
            radius,
            capacity,
            locked,
        }
    }

    #[must_use]
    pub const fn color_index(&self) -> u16 {
        self.color_index
    }

    /// `None` selects the consumer's default radius.
    #[must_use]
    pub const fn radius(&self) -> Option<u8> {
        self.radius
    }

    #[must_use]
    pub const fn capacity(&self) -> CollectCapacity {
        self.capacity
    }

    #[must_use]
    pub const fn is_locked(&self) -> bool {
        self.locked
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    collect_layers: Vec<CollectLayer>,
}

impl Block {
    #[must_use]
    pub fn new(collect_layers: Vec<CollectLayer>) -> Self {
        Self { collect_layers }
    }

    #[must_use]
    pub fn collect_layers(&self) -> &[CollectLayer] {
        &self.collect_layers
    }
}

/// A row-major, lower-left-origin tile with bounded color indices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BlindTile {
    pixels_per_cell: u8,
    bits: Vec<u8>,
    colors: Vec<u8>,
}

#[derive(Deserialize)]
struct SerializedBlindTile {
    pixels_per_cell: u8,
    bits: Vec<u8>,
    #[serde(default)]
    colors: Option<Vec<u8>>,
}

impl<'de> Deserialize<'de> for BlindTile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let tile = SerializedBlindTile::deserialize(deserializer)?;
        match tile.colors {
            Some(colors) => {
                let restored =
                    Self::from_colors(tile.pixels_per_cell, colors).map_err(D::Error::custom)?;
                if restored.bits != tile.bits {
                    return Err(D::Error::custom(EntityError::BlindTileBitsMismatch));
                }
                Ok(restored)
            }
            None => Self::new(tile.pixels_per_cell, tile.bits).map_err(D::Error::custom),
        }
    }
}

impl BlindTile {
    pub fn new(pixels_per_cell: u8, bits: Vec<u8>) -> Result<Self, EntityError> {
        validate_pixels_per_cell(pixels_per_cell)?;
        validate_tile_bits(pixels_per_cell, &bits)?;
        let pixel_count = tile_pixel_count(pixels_per_cell);
        let colors = (0..pixel_count)
            .map(|index| u8::from(bit_is_set(&bits, index)))
            .collect();
        Ok(Self {
            pixels_per_cell,
            bits,
            colors,
        })
    }

    pub fn from_colors(pixels_per_cell: u8, colors: Vec<u8>) -> Result<Self, EntityError> {
        validate_pixels_per_cell(pixels_per_cell)?;
        let expected = tile_pixel_count(pixels_per_cell);
        if colors.len() != expected {
            return Err(EntityError::BlindTileColorCount {
                expected,
                actual: colors.len(),
            });
        }
        if let Some(color_index) = colors.iter().copied().find(|color| *color > 10) {
            return Err(EntityError::InvalidBlindTileColor(color_index));
        }

        Ok(Self::from_valid_colors(pixels_per_cell, colors))
    }

    pub fn empty(pixels_per_cell: u8) -> Result<Self, EntityError> {
        validate_pixels_per_cell(pixels_per_cell)?;
        Self::from_colors(pixels_per_cell, vec![0; tile_pixel_count(pixels_per_cell)])
    }

    #[must_use]
    pub const fn pixels_per_cell(&self) -> u8 {
        self.pixels_per_cell
    }

    /// Legacy occupancy bits, derived from non-zero colors.
    #[must_use]
    pub fn bits(&self) -> &[u8] {
        &self.bits
    }

    #[must_use]
    pub fn colors(&self) -> &[u8] {
        &self.colors
    }

    #[must_use]
    pub fn pixel(&self, x: u8, y: u8) -> Option<bool> {
        self.color(x, y).map(|color| color != 0)
    }

    #[must_use]
    pub fn color(&self, x: u8, y: u8) -> Option<u8> {
        let index = self.pixel_index(x, y)?;
        self.colors.get(index).copied()
    }

    pub(crate) fn set_color(&mut self, x: u8, y: u8, color_index: u8) -> bool {
        let Some(index) = self.pixel_index(x, y) else {
            return false;
        };
        if self.colors[index] == color_index {
            return false;
        }
        self.colors[index] = color_index;
        set_bit(&mut self.bits, index, color_index != 0);
        true
    }

    pub(crate) fn has_non_default_colors(&self) -> bool {
        self.colors.iter().copied().any(|color| color > 1)
    }

    fn rotated_clockwise(&self) -> Self {
        self.transform_pixels(|size, x, y| (size - 1 - y, x))
    }

    fn flipped_horizontal(&self) -> Self {
        self.transform_pixels(|size, x, y| (size - 1 - x, y))
    }

    fn transform_pixels(&self, transform: impl Fn(u8, u8, u8) -> (u8, u8)) -> Self {
        let mut colors = vec![0_u8; self.colors.len()];
        for y in 0..self.pixels_per_cell {
            for x in 0..self.pixels_per_cell {
                let (next_x, next_y) = transform(self.pixels_per_cell, x, y);
                let next_index =
                    usize::from(next_x) + usize::from(next_y) * usize::from(self.pixels_per_cell);
                colors[next_index] = self.color(x, y).unwrap_or_default();
            }
        }
        Self::from_valid_colors(self.pixels_per_cell, colors)
    }

    fn pixel_index(&self, x: u8, y: u8) -> Option<usize> {
        if x >= self.pixels_per_cell || y >= self.pixels_per_cell {
            return None;
        }
        Some(usize::from(x) + usize::from(y) * usize::from(self.pixels_per_cell))
    }

    fn from_valid_colors(pixels_per_cell: u8, colors: Vec<u8>) -> Self {
        let mut bits = vec![0_u8; tile_byte_len(pixels_per_cell)];
        for (index, color) in colors.iter().enumerate() {
            set_bit(&mut bits, index, *color != 0);
        }
        Self {
            pixels_per_cell,
            bits,
            colors,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Blind {
    pixels_per_cell: u8,
    tiles: Vec<BlindTile>,
    guides: Vec<BlindGuide>,
}

#[derive(Deserialize)]
struct SerializedBlind {
    pixels_per_cell: u8,
    tiles: Vec<BlindTile>,
    #[serde(default)]
    guides: Vec<BlindGuide>,
}

impl<'de> Deserialize<'de> for Blind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let blind = SerializedBlind::deserialize(deserializer)?;
        Self::with_guides(blind.pixels_per_cell, blind.tiles, blind.guides)
            .map_err(D::Error::custom)
    }
}

impl Blind {
    pub fn new(pixels_per_cell: u8, tiles: Vec<BlindTile>) -> Result<Self, EntityError> {
        Self::with_guides(pixels_per_cell, tiles, Vec::new())
    }

    pub fn with_guides(
        pixels_per_cell: u8,
        tiles: Vec<BlindTile>,
        mut guides: Vec<BlindGuide>,
    ) -> Result<Self, EntityError> {
        validate_pixels_per_cell(pixels_per_cell)?;
        if tiles.len() > usize::from(MAX_SHAPE_AREA) {
            return Err(EntityError::BlindTileLimit {
                actual: tiles.len(),
            });
        }
        if guides.len() > MAX_BLIND_GUIDES {
            return Err(BrushError::GuideSetTooLarge {
                actual: guides.len(),
            }
            .into());
        }
        if let Some(tile) = tiles
            .iter()
            .find(|tile| tile.pixels_per_cell != pixels_per_cell)
        {
            return Err(EntityError::BlindTileResolution {
                expected: pixels_per_cell,
                actual: tile.pixels_per_cell,
            });
        }
        guides.sort_unstable();
        guides.dedup();

        Ok(Self {
            pixels_per_cell,
            tiles,
            guides,
        })
    }

    #[must_use]
    pub const fn pixels_per_cell(&self) -> u8 {
        self.pixels_per_cell
    }

    /// Tiles correspond one-for-one with `Shape::occupied_cells()` order.
    #[must_use]
    pub fn tiles(&self) -> &[BlindTile] {
        &self.tiles
    }

    #[must_use]
    pub fn guides(&self) -> &[BlindGuide] {
        &self.guides
    }

    #[must_use]
    pub fn tile_for_cell(&self, shape: Shape, cell: ShapeCell) -> Option<&BlindTile> {
        self.tile_index(shape, cell)
            .and_then(|index| self.tiles.get(index))
    }

    #[must_use]
    pub fn color_at(&self, shape: Shape, pixel: BlindPixel) -> Option<u8> {
        let (tile_index, x, y) = self.pixel_location(shape, pixel)?;
        self.tiles.get(tile_index)?.color(x, y)
    }

    #[must_use]
    pub fn has_guide(&self, guide: BlindGuide) -> bool {
        self.guides.binary_search(&guide).is_ok()
    }

    pub fn apply_operation(
        &self,
        shape: Shape,
        operation: &BlindBrushOperation,
    ) -> Result<Self, BrushError> {
        let mut next = self.clone();
        match operation {
            BlindBrushOperation::PaintStroke {
                color_index,
                stroke,
            } => {
                validate_color_index(*color_index)?;
                for pixel in stroke.pixels() {
                    next.set_pixel_color(shape, *pixel, *color_index);
                }
            }
            BlindBrushOperation::EraseStroke { stroke } => {
                for pixel in stroke.pixels() {
                    next.set_pixel_color(shape, *pixel, 0);
                }
            }
            BlindBrushOperation::FloodFill { start, color_index } => {
                validate_color_index(*color_index)?;
                next.flood_fill(shape, *start, *color_index);
            }
            BlindBrushOperation::AddGuides { guides } => {
                for guide in guides.guides() {
                    if next.guide_is_in_footprint(shape, *guide) {
                        match next.guides.binary_search(guide) {
                            Ok(_) => {}
                            Err(index) => next.guides.insert(index, *guide),
                        }
                    }
                }
            }
            BlindBrushOperation::RemoveGuides { guides } => {
                for guide in guides.guides() {
                    if let Ok(index) = next.guides.binary_search(guide) {
                        next.guides.remove(index);
                    }
                }
            }
        }
        Ok(next)
    }

    fn flood_fill(&mut self, shape: Shape, start: BlindPixel, color_index: u8) {
        let Some(source_color) = self.color_at(shape, start) else {
            return;
        };
        if source_color == color_index {
            return;
        }

        let mut pending = VecDeque::from([start]);
        let mut visited = BTreeSet::from([start]);
        while let Some(pixel) = pending.pop_front() {
            if self.color_at(shape, pixel) != Some(source_color) {
                continue;
            }
            self.set_pixel_color(shape, pixel, color_index);

            for neighbor in pixel_neighbors(pixel).into_iter().flatten() {
                if visited.contains(&neighbor)
                    || self.color_at(shape, neighbor) != Some(source_color)
                {
                    continue;
                }
                let guide = BlindGuide::from_transformed(pixel, neighbor);
                if self.has_guide(guide) {
                    continue;
                }
                visited.insert(neighbor);
                pending.push_back(neighbor);
            }
        }
    }

    fn set_pixel_color(&mut self, shape: Shape, pixel: BlindPixel, color_index: u8) -> bool {
        let Some((tile_index, x, y)) = self.pixel_location(shape, pixel) else {
            return false;
        };
        self.tiles[tile_index].set_color(x, y, color_index)
    }

    fn set_tile(
        &mut self,
        shape: Shape,
        cell: ShapeCell,
        tile: BlindTile,
    ) -> Result<(), EntityError> {
        if tile.pixels_per_cell != self.pixels_per_cell {
            return Err(EntityError::BlindTileResolution {
                expected: self.pixels_per_cell,
                actual: tile.pixels_per_cell,
            });
        }
        let index = self
            .tile_index(shape, cell)
            .ok_or(BrushError::InvalidTilePatchCell(cell))?;
        self.tiles[index] = tile;
        Ok(())
    }

    fn set_guide_presence(
        &mut self,
        shape: Shape,
        guide: BlindGuide,
        present: bool,
    ) -> Result<(), BrushError> {
        if !self.guide_is_in_footprint(shape, guide) {
            return Err(BrushError::GuideOutsideFootprint(guide));
        }
        match (self.guides.binary_search(&guide), present) {
            (Err(index), true) => self.guides.insert(index, guide),
            (Ok(index), false) => {
                self.guides.remove(index);
            }
            _ => {}
        }
        Ok(())
    }

    fn tile_index(&self, shape: Shape, cell: ShapeCell) -> Option<usize> {
        shape.occupied_cells().position(|occupied| occupied == cell)
    }

    fn pixel_location(&self, shape: Shape, pixel: BlindPixel) -> Option<(usize, u8, u8)> {
        let resolution = u16::from(self.pixels_per_cell);
        let cell_x = pixel.x / resolution;
        let cell_y = pixel.y / resolution;
        if cell_x >= u16::from(shape.width()) || cell_y >= u16::from(shape.height()) {
            return None;
        }
        let cell = ShapeCell::new(cell_x as u8, cell_y as u8);
        let tile_index = self.tile_index(shape, cell)?;
        Some((
            tile_index,
            (pixel.x % resolution) as u8,
            (pixel.y % resolution) as u8,
        ))
    }

    fn guide_is_in_footprint(&self, shape: Shape, guide: BlindGuide) -> bool {
        self.pixel_location(shape, guide.first()).is_some()
            && self.pixel_location(shape, guide.second()).is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaceableEntityKind {
    Block(Block),
    Blind(Blind),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlaceableEntity {
    id: EntityId,
    origin: GridPoint,
    shape: Shape,
    kind: PlaceableEntityKind,
}

#[derive(Deserialize)]
struct SerializedPlaceableEntity {
    id: EntityId,
    origin: GridPoint,
    shape: Shape,
    kind: PlaceableEntityKind,
}

impl<'de> Deserialize<'de> for PlaceableEntity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entity = SerializedPlaceableEntity::deserialize(deserializer)?;
        Self::new(entity.id, entity.origin, entity.shape, entity.kind).map_err(D::Error::custom)
    }
}

impl PlaceableEntity {
    pub fn new(
        id: impl Into<EntityId>,
        origin: GridPoint,
        shape: Shape,
        kind: PlaceableEntityKind,
    ) -> Result<Self, EntityError> {
        validate_kind(shape, &kind)?;
        Ok(Self {
            id: id.into(),
            origin,
            shape,
            kind,
        })
    }

    pub fn block(
        id: impl Into<EntityId>,
        origin: GridPoint,
        shape: Shape,
        block: Block,
    ) -> Result<Self, EntityError> {
        Self::new(id, origin, shape, PlaceableEntityKind::Block(block))
    }

    pub fn blind(
        id: impl Into<EntityId>,
        origin: GridPoint,
        shape: Shape,
        blind: Blind,
    ) -> Result<Self, EntityError> {
        Self::new(id, origin, shape, PlaceableEntityKind::Blind(blind))
    }

    #[must_use]
    pub const fn id(&self) -> &EntityId {
        &self.id
    }

    #[must_use]
    pub const fn origin(&self) -> GridPoint {
        self.origin
    }

    #[must_use]
    pub const fn shape(&self) -> Shape {
        self.shape
    }

    #[must_use]
    pub const fn kind(&self) -> &PlaceableEntityKind {
        &self.kind
    }

    #[must_use]
    pub const fn as_blind(&self) -> Option<&Blind> {
        match &self.kind {
            PlaceableEntityKind::Blind(blind) => Some(blind),
            PlaceableEntityKind::Block(_) => None,
        }
    }

    #[must_use]
    pub fn moved_to(&self, origin: GridPoint) -> Self {
        Self {
            origin,
            ..self.clone()
        }
    }

    pub fn apply_blind_operation(
        &self,
        operation: &BlindBrushOperation,
    ) -> Result<Self, EntityError> {
        let PlaceableEntityKind::Blind(blind) = &self.kind else {
            return Err(EntityError::NotBlind(self.id.clone()));
        };
        let mut next = self.clone();
        next.kind = PlaceableEntityKind::Blind(blind.apply_operation(self.shape, operation)?);
        Ok(next)
    }

    pub(crate) fn restore_blind_patch(
        &self,
        tile_patches: &[BlindTilePatch],
        guide_patches: &[BlindGuidePatch],
    ) -> Result<Self, EntityError> {
        if tile_patches.len() > usize::from(MAX_SHAPE_AREA) {
            return Err(BrushError::TilePatchSetTooLarge {
                actual: tile_patches.len(),
            }
            .into());
        }
        if guide_patches.len() > MAX_BLIND_GUIDES {
            return Err(BrushError::GuideSetTooLarge {
                actual: guide_patches.len(),
            }
            .into());
        }
        let mut tile_cells = BTreeSet::new();
        for patch in tile_patches {
            if !tile_cells.insert(patch.cell) {
                return Err(BrushError::DuplicateTilePatch(patch.cell).into());
            }
        }
        let mut guide_edges = BTreeSet::new();
        for patch in guide_patches {
            if !guide_edges.insert(patch.guide) {
                return Err(BrushError::DuplicateGuidePatch(patch.guide).into());
            }
        }

        let mut next = self.clone();
        let PlaceableEntityKind::Blind(blind) = &mut next.kind else {
            return Err(EntityError::NotBlind(self.id.clone()));
        };
        for patch in tile_patches {
            blind.set_tile(self.shape, patch.cell, patch.tile.clone())?;
        }
        for patch in guide_patches {
            blind.set_guide_presence(self.shape, patch.guide, patch.present)?;
        }
        Ok(next)
    }

    #[must_use]
    pub fn rotated_clockwise(&self) -> Self {
        let old_shape = self.shape;
        let kind = match &self.kind {
            PlaceableEntityKind::Block(block) => PlaceableEntityKind::Block(block.clone()),
            PlaceableEntityKind::Blind(blind) => {
                let mut transformed: Vec<_> = old_shape
                    .occupied_cells()
                    .zip(&blind.tiles)
                    .map(|(cell, tile)| {
                        (
                            old_shape.rotate_cell_clockwise(cell),
                            tile.rotated_clockwise(),
                        )
                    })
                    .collect();
                transformed.sort_by_key(|(cell, _)| (cell.y, cell.x));
                let pixel_height = u16::from(old_shape.height()) * u16::from(blind.pixels_per_cell);
                let mut guides: Vec<_> = blind
                    .guides
                    .iter()
                    .map(|guide| {
                        BlindGuide::from_transformed(
                            BlindPixel::new(pixel_height - 1 - guide.first().y, guide.first().x),
                            BlindPixel::new(pixel_height - 1 - guide.second().y, guide.second().x),
                        )
                    })
                    .collect();
                guides.sort_unstable();
                PlaceableEntityKind::Blind(Blind {
                    pixels_per_cell: blind.pixels_per_cell,
                    tiles: transformed.into_iter().map(|(_, tile)| tile).collect(),
                    guides,
                })
            }
        };

        Self {
            shape: old_shape.rotated_clockwise(),
            kind,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn flipped_horizontal(&self) -> Self {
        let old_shape = self.shape;
        let kind = match &self.kind {
            PlaceableEntityKind::Block(block) => PlaceableEntityKind::Block(block.clone()),
            PlaceableEntityKind::Blind(blind) => {
                let mut transformed: Vec<_> = old_shape
                    .occupied_cells()
                    .zip(&blind.tiles)
                    .map(|(cell, tile)| {
                        (
                            old_shape.flip_cell_horizontal(cell),
                            tile.flipped_horizontal(),
                        )
                    })
                    .collect();
                transformed.sort_by_key(|(cell, _)| (cell.y, cell.x));
                let pixel_width = u16::from(old_shape.width()) * u16::from(blind.pixels_per_cell);
                let mut guides: Vec<_> = blind
                    .guides
                    .iter()
                    .map(|guide| {
                        BlindGuide::from_transformed(
                            BlindPixel::new(pixel_width - 1 - guide.first().x, guide.first().y),
                            BlindPixel::new(pixel_width - 1 - guide.second().x, guide.second().y),
                        )
                    })
                    .collect();
                guides.sort_unstable();
                PlaceableEntityKind::Blind(Blind {
                    pixels_per_cell: blind.pixels_per_cell,
                    tiles: transformed.into_iter().map(|(_, tile)| tile).collect(),
                    guides,
                })
            }
        };

        Self {
            shape: old_shape.flipped_horizontal(),
            kind,
            ..self.clone()
        }
    }
}

fn validate_kind(shape: Shape, kind: &PlaceableEntityKind) -> Result<(), EntityError> {
    match kind {
        PlaceableEntityKind::Block(_) => {
            if shape.width() > MAX_BLOCK_SHAPE_AXIS || shape.height() > MAX_BLOCK_SHAPE_AXIS {
                return Err(EntityError::BlockShapeTooLarge {
                    width: shape.width(),
                    height: shape.height(),
                });
            }
        }
        PlaceableEntityKind::Blind(blind) => {
            let expected = shape.occupied_count() as usize;
            if blind.tiles.len() != expected {
                return Err(EntityError::BlindTileCount {
                    expected,
                    actual: blind.tiles.len(),
                });
            }
            if let Some(guide) = blind
                .guides
                .iter()
                .find(|guide| !blind.guide_is_in_footprint(shape, **guide))
            {
                return Err(BrushError::GuideOutsideFootprint(*guide).into());
            }
        }
    }
    Ok(())
}

fn validate_pixels_per_cell(pixels_per_cell: u8) -> Result<(), EntityError> {
    if !(MIN_BLIND_PIXELS_PER_CELL..=MAX_BLIND_PIXELS_PER_CELL).contains(&pixels_per_cell) {
        return Err(EntityError::InvalidPixelsPerCell(pixels_per_cell));
    }
    Ok(())
}

fn validate_tile_bits(pixels_per_cell: u8, bits: &[u8]) -> Result<(), EntityError> {
    let expected = tile_byte_len(pixels_per_cell);
    if bits.len() != expected {
        return Err(EntityError::BlindTileByteCount {
            expected,
            actual: bits.len(),
        });
    }
    let remainder = tile_pixel_count(pixels_per_cell) % 8;
    if remainder != 0 {
        let valid_last_byte = (1_u8 << remainder) - 1;
        if bits.last().is_some_and(|byte| byte & !valid_last_byte != 0) {
            return Err(EntityError::BlindTilePadding);
        }
    }
    Ok(())
}

fn pixel_neighbors(pixel: BlindPixel) -> [Option<BlindPixel>; 4] {
    [
        pixel.x.checked_sub(1).map(|x| BlindPixel::new(x, pixel.y)),
        pixel.x.checked_add(1).map(|x| BlindPixel::new(x, pixel.y)),
        pixel.y.checked_sub(1).map(|y| BlindPixel::new(pixel.x, y)),
        pixel.y.checked_add(1).map(|y| BlindPixel::new(pixel.x, y)),
    ]
}

fn bit_is_set(bits: &[u8], index: usize) -> bool {
    bits[index / 8] & (1_u8 << (index % 8)) != 0
}

fn set_bit(bits: &mut [u8], index: usize, set: bool) {
    let mask = 1_u8 << (index % 8);
    if set {
        bits[index / 8] |= mask;
    } else {
        bits[index / 8] &= !mask;
    }
}

const fn tile_pixel_count(pixels_per_cell: u8) -> usize {
    pixels_per_cell as usize * pixels_per_cell as usize
}

const fn tile_byte_len(pixels_per_cell: u8) -> usize {
    tile_pixel_count(pixels_per_cell).div_ceil(8)
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum EntityError {
    #[error(
        "block shape {width} x {height} exceeds the {MAX_BLOCK_SHAPE_AXIS} x {MAX_BLOCK_SHAPE_AXIS} limit"
    )]
    BlockShapeTooLarge { width: u8, height: u8 },

    #[error(
        "Blind pixels-per-cell must be in {MIN_BLIND_PIXELS_PER_CELL}..={MAX_BLIND_PIXELS_PER_CELL}, got {0}"
    )]
    InvalidPixelsPerCell(u8),

    #[error("Blind tile has {actual} bytes but {expected} were expected")]
    BlindTileByteCount { expected: usize, actual: usize },

    #[error("Blind tile has non-zero padding bits")]
    BlindTilePadding,

    #[error("Blind tile has {actual} colors but {expected} were expected")]
    BlindTileColorCount { expected: usize, actual: usize },

    #[error("Blind tile color index must be in 0..=10, got {0}")]
    InvalidBlindTileColor(u8),

    #[error("Blind tile occupancy bits do not match its colors")]
    BlindTileBitsMismatch,

    #[error("Blind tile resolution is {actual}, expected {expected}")]
    BlindTileResolution { expected: u8, actual: u8 },

    #[error("Blind has {actual} tiles but its shape has {expected} occupied cells")]
    BlindTileCount { expected: usize, actual: usize },

    #[error("Blind has {actual} tiles, exceeding the {MAX_SHAPE_AREA}-tile limit")]
    BlindTileLimit { actual: usize },

    #[error("entity '{0}' is not a Blind")]
    NotBlind(EntityId),

    #[error(transparent)]
    InvalidBrush(#[from] BrushError),
}
