use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::brush::validate_color_index;
use crate::{
    BlindBrushOperation, BlindGuide, BlindGuidePatch, BlindPixel, BlindTilePatch, BrushError,
    EntityId, GridPoint, MAX_BLIND_GUIDES, MAX_BLOCK_SHAPE_AXIS, MAX_SHAPE_AREA, PoolBoundary,
    Shape, ShapeCell,
};

pub const MIN_BLIND_PIXELS_PER_CELL: u8 = 1;
pub const MAX_BLIND_PIXELS_PER_CELL: u8 = 32;
/// Maximum representable collect-layer count in the legacy level format.
pub const MAX_COLLECT_LAYERS: usize = u16::MAX as usize;
/// The legacy sentinel for a disabled collect color.
pub const DISABLED_COLLECT_COLOR_INDEX: u16 = u16::MAX;
pub const MAX_COLLECT_COLOR_INDEX: u16 = 15;
pub const MAX_COLLECT_CAPACITY: u32 = i32::MAX as u32;

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

    /// Resamples this tile by choosing the nearest source pixel center.
    /// Exact ties select the source pixel with the higher coordinate.
    pub fn resampled(&self, pixels_per_cell: u8) -> Result<Self, EntityError> {
        validate_pixels_per_cell(pixels_per_cell)?;
        if self.pixels_per_cell == pixels_per_cell {
            return Ok(self.clone());
        }

        let mut colors = Vec::with_capacity(tile_pixel_count(pixels_per_cell));
        for y in 0..pixels_per_cell {
            let source_y = nearest_source_pixel(y, self.pixels_per_cell, pixels_per_cell);
            for x in 0..pixels_per_cell {
                let source_x = nearest_source_pixel(x, self.pixels_per_cell, pixels_per_cell);
                colors.push(self.color(source_x, source_y).unwrap_or_default());
            }
        }
        Ok(Self::from_valid_colors(pixels_per_cell, colors))
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

/// Maximum number of explicit regions; group zero is always the implicit Default.
pub const MAX_POOL_DISTRIBUTION_GROUPS: usize = 64;
pub const MAX_POOL_DISTRIBUTION_GROUP_NAME_CHARS: usize = 80;

/// Persistent pixel membership, independent of colors, connectivity, and Blind guides.
/// Pixels use the same lower-left local coordinates as Blind brush pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolDistributionGroup {
    pub id: u32,
    pub name: String,
    pub pixels: Vec<BlindPixel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Blind {
    pixels_per_cell: u8,
    tiles: Vec<BlindTile>,
    guides: Vec<BlindGuide>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    distribution_groups: Vec<PoolDistributionGroup>,
    #[serde(default, skip_serializing_if = "PoolBoundary::is_default")]
    boundary: PoolBoundary,
}

#[derive(Deserialize)]
struct SerializedBlind {
    pixels_per_cell: u8,
    tiles: Vec<BlindTile>,
    #[serde(default)]
    guides: Vec<BlindGuide>,
    #[serde(default)]
    distribution_groups: Vec<PoolDistributionGroup>,
    #[serde(default)]
    boundary: PoolBoundary,
}

impl<'de> Deserialize<'de> for Blind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let blind = SerializedBlind::deserialize(deserializer)?;
        let mut restored = Self::with_guides(blind.pixels_per_cell, blind.tiles, blind.guides)
            .map_err(D::Error::custom)?;
        restored.distribution_groups =
            canonical_distribution_groups(blind.distribution_groups).map_err(D::Error::custom)?;
        // The containing PlaceableEntity validates membership against its shape.
        validate_pool_boundary(blind.boundary).map_err(D::Error::custom)?;
        restored.boundary = blind.boundary;
        Ok(restored)
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
            distribution_groups: Vec::new(),
            boundary: PoolBoundary::default(),
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
    pub const fn boundary(&self) -> PoolBoundary {
        self.boundary
    }

    /// Explicit boundary values are absolute pixels, including after resampling.
    pub fn with_boundary(&self, boundary: PoolBoundary) -> Result<Self, EntityError> {
        validate_pool_boundary(boundary)?;
        let mut next = self.clone();
        next.boundary = boundary;
        Ok(next)
    }

    /// Explicit groups sorted by ID, with sorted pixel membership. Unlisted
    /// pixels (including unpainted pixels) belong to implicit Default group zero.
    #[must_use]
    pub fn distribution_groups(&self) -> &[PoolDistributionGroup] {
        &self.distribution_groups
    }

    /// Replaces all explicit groups atomically, normalizing names and order.
    /// Empty groups are retained so their identity survives cropping/resampling.
    pub fn with_distribution_groups(
        &self,
        shape: Shape,
        groups: Vec<PoolDistributionGroup>,
    ) -> Result<Self, EntityError> {
        let groups = canonical_distribution_groups(groups)?;
        let mut next = self.clone();
        next.distribution_groups = groups;
        validate_blind(shape, &next)?;
        Ok(next)
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

    /// Resamples every occupied tile independently by nearest source pixel center.
    ///
    /// Guides are projected onto destination pixel adjacencies. A destination
    /// edge is retained when its sampled source path crosses a source guide.
    /// Source edges not sampled by any destination adjacency are dropped, and
    /// multiple source edges may merge into one destination edge. This keeps
    /// the result deterministic, valid, and bounded by the destination graph.
    /// Distribution membership samples the same source centers as tile colors,
    /// even for color zero. Groups with no sampled pixels retain their ID/name.
    pub fn resampled(&self, shape: Shape, pixels_per_cell: u8) -> Result<Self, EntityError> {
        validate_pixels_per_cell(pixels_per_cell)?;
        if self.pixels_per_cell == pixels_per_cell {
            return Ok(self.clone());
        }

        let tiles = self
            .tiles
            .iter()
            .map(|tile| tile.resampled(pixels_per_cell))
            .collect::<Result<Vec<_>, _>>()?;
        let guides = self.resampled_guides(shape, pixels_per_cell);
        let groups = self.resampled_distribution_groups(shape, pixels_per_cell);
        Self::with_guides(pixels_per_cell, tiles, guides)?
            .with_distribution_groups(shape, groups)?
            .with_boundary(self.boundary)
    }

    #[must_use]
    pub fn fill_region(&self, shape: Shape, start: BlindPixel) -> Vec<BlindPixel> {
        let Some(source_color) = self.color_at(shape, start) else {
            return Vec::new();
        };

        let mut pending = VecDeque::from([start]);
        let mut visited = BTreeSet::from([start]);
        let mut region = Vec::new();
        while let Some(pixel) = pending.pop_front() {
            if self.color_at(shape, pixel) != Some(source_color) {
                continue;
            }
            region.push(pixel);

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
        region
    }

    #[must_use]
    pub fn paintable_partition(&self, shape: Shape, start: BlindPixel) -> Vec<BlindPixel> {
        if self.color_at(shape, start).is_none() {
            return Vec::new();
        }

        let mut pending = VecDeque::from([start]);
        let mut visited = BTreeSet::from([start]);
        let mut region = Vec::new();
        while let Some(pixel) = pending.pop_front() {
            if self.color_at(shape, pixel).is_none() {
                continue;
            }
            region.push(pixel);

            for neighbor in pixel_neighbors(pixel).into_iter().flatten() {
                if visited.contains(&neighbor) || self.color_at(shape, neighbor).is_none() {
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
        region
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
        for pixel in self.fill_region(shape, start) {
            self.set_pixel_color(shape, pixel, color_index);
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

    fn resampled_guides(&self, shape: Shape, pixels_per_cell: u8) -> Vec<BlindGuide> {
        let width = u16::from(shape.width()) * u16::from(pixels_per_cell);
        let height = u16::from(shape.height()) * u16::from(pixels_per_cell);
        let mut guides = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let first = BlindPixel::new(x, y);
                if !pixel_is_in_footprint(shape, first, pixels_per_cell) {
                    continue;
                }
                for second in [
                    x.checked_add(1)
                        .filter(|next_x| *next_x < width)
                        .map(|next_x| BlindPixel::new(next_x, y)),
                    y.checked_add(1)
                        .filter(|next_y| *next_y < height)
                        .map(|next_y| BlindPixel::new(x, next_y)),
                ]
                .into_iter()
                .flatten()
                {
                    if !pixel_is_in_footprint(shape, second, pixels_per_cell) {
                        continue;
                    }
                    let source_first =
                        resampled_source_pixel(first, self.pixels_per_cell, pixels_per_cell);
                    let source_second =
                        resampled_source_pixel(second, self.pixels_per_cell, pixels_per_cell);
                    if self.source_path_has_guide(source_first, source_second) {
                        guides.push(BlindGuide::from_transformed(first, second));
                    }
                }
            }
        }
        guides
    }

    fn resampled_distribution_groups(
        &self,
        shape: Shape,
        pixels_per_cell: u8,
    ) -> Vec<PoolDistributionGroup> {
        let mut groups = self.distribution_groups.clone();
        if groups.is_empty() {
            return groups;
        }
        let membership: BTreeMap<_, _> = groups
            .iter()
            .enumerate()
            .flat_map(|(index, group)| group.pixels.iter().map(move |pixel| (*pixel, index)))
            .collect();
        for group in &mut groups {
            group.pixels.clear();
        }
        for cell in shape.occupied_cells() {
            for y in 0..u16::from(pixels_per_cell) {
                for x in 0..u16::from(pixels_per_cell) {
                    let destination = BlindPixel::new(
                        u16::from(cell.x) * u16::from(pixels_per_cell) + x,
                        u16::from(cell.y) * u16::from(pixels_per_cell) + y,
                    );
                    let source =
                        resampled_source_pixel(destination, self.pixels_per_cell, pixels_per_cell);
                    if let Some(index) = membership.get(&source) {
                        groups[*index].pixels.push(destination);
                    }
                }
            }
        }
        groups
    }

    fn transformed_distribution_groups(
        &self,
        transform: impl Fn(BlindPixel) -> BlindPixel,
    ) -> Vec<PoolDistributionGroup> {
        self.distribution_groups
            .iter()
            .map(|group| {
                let mut pixels: Vec<_> = group.pixels.iter().copied().map(&transform).collect();
                pixels.sort_unstable();
                PoolDistributionGroup {
                    id: group.id,
                    name: group.name.clone(),
                    pixels,
                }
            })
            .collect()
    }

    fn source_path_has_guide(&self, first: BlindPixel, second: BlindPixel) -> bool {
        if first.y == second.y {
            return (first.x..second.x).any(|x| {
                self.has_guide(BlindGuide::from_transformed(
                    BlindPixel::new(x, first.y),
                    BlindPixel::new(x + 1, first.y),
                ))
            });
        }
        (first.y..second.y).any(|y| {
            self.has_guide(BlindGuide::from_transformed(
                BlindPixel::new(first.x, y),
                BlindPixel::new(first.x, y + 1),
            ))
        })
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

    pub fn with_block_collect_layers(
        &self,
        layers: Vec<CollectLayer>,
    ) -> Result<Self, EntityError> {
        if !matches!(&self.kind, PlaceableEntityKind::Block(_)) {
            return Err(EntityError::NotBlock(self.id.clone()));
        }
        Self::block(self.id.clone(), self.origin, self.shape, Block::new(layers))
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

    pub fn with_pool_distribution_groups(
        &self,
        groups: Vec<PoolDistributionGroup>,
    ) -> Result<Self, EntityError> {
        let PlaceableEntityKind::Blind(blind) = &self.kind else {
            return Err(EntityError::NotBlind(self.id.clone()));
        };
        Self::blind(
            self.id.clone(),
            self.origin,
            self.shape,
            blind.with_distribution_groups(self.shape, groups)?,
        )
    }

    pub fn resampled_blind(&self, pixels_per_cell: u8) -> Result<Self, EntityError> {
        let PlaceableEntityKind::Blind(blind) = &self.kind else {
            return Err(EntityError::NotBlind(self.id.clone()));
        };
        let mut next = self.clone();
        next.kind = PlaceableEntityKind::Blind(blind.resampled(self.shape, pixels_per_cell)?);
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
                let distribution_groups = blind.transformed_distribution_groups(|pixel| {
                    BlindPixel::new(pixel_height - 1 - pixel.y, pixel.x)
                });
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
                    distribution_groups,
                    boundary: blind.boundary,
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
                let distribution_groups = blind.transformed_distribution_groups(|pixel| {
                    BlindPixel::new(pixel_width - 1 - pixel.x, pixel.y)
                });
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
                    distribution_groups,
                    boundary: blind.boundary,
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
        PlaceableEntityKind::Block(block) => {
            if shape.width() > MAX_BLOCK_SHAPE_AXIS || shape.height() > MAX_BLOCK_SHAPE_AXIS {
                return Err(EntityError::BlockShapeTooLarge {
                    width: shape.width(),
                    height: shape.height(),
                });
            }
            validate_collect_layers(block.collect_layers())?;
        }
        PlaceableEntityKind::Blind(blind) => validate_blind(shape, blind)?,
    }
    Ok(())
}

fn validate_blind(shape: Shape, blind: &Blind) -> Result<(), EntityError> {
    let expected = shape.occupied_count() as usize;
    if blind.tiles.len() != expected {
        return Err(EntityError::BlindTileCount {
            expected,
            actual: blind.tiles.len(),
        });
    }
    validate_distribution_group_pixels(shape, blind.pixels_per_cell, &blind.distribution_groups)?;
    validate_pool_boundary(blind.boundary)?;
    if let Some(guide) = blind
        .guides
        .iter()
        .find(|guide| !blind.guide_is_in_footprint(shape, **guide))
    {
        return Err(BrushError::GuideOutsideFootprint(*guide).into());
    }
    Ok(())
}

fn canonical_distribution_groups(
    mut groups: Vec<PoolDistributionGroup>,
) -> Result<Vec<PoolDistributionGroup>, EntityError> {
    if groups.len() > MAX_POOL_DISTRIBUTION_GROUPS {
        return Err(EntityError::PoolDistributionGroupLimit {
            actual: groups.len(),
        });
    }
    let mut ids = BTreeSet::new();
    let mut pixels = BTreeSet::new();
    for group in &mut groups {
        if group.id == 0 {
            return Err(EntityError::ReservedPoolDistributionGroupId);
        }
        if !ids.insert(group.id) {
            return Err(EntityError::DuplicatePoolDistributionGroupId(group.id));
        }
        group.name = group.name.trim().to_owned();
        if group.name.is_empty()
            || group.name.chars().count() > MAX_POOL_DISTRIBUTION_GROUP_NAME_CHARS
        {
            return Err(EntityError::InvalidPoolDistributionGroupName(group.id));
        }
        // This bounds standalone Blind deserialization too, before the containing
        // entity can check the actual footprint. Never silently deduplicate input.
        if group.pixels.len()
            > usize::from(MAX_SHAPE_AREA) * tile_pixel_count(MAX_BLIND_PIXELS_PER_CELL)
        {
            return Err(EntityError::PoolDistributionGroupPixelLimit { id: group.id });
        }
        for pixel in &group.pixels {
            if !pixels.insert(*pixel) {
                return Err(EntityError::DuplicatePoolDistributionPixel(*pixel));
            }
        }
        group.pixels.sort_unstable();
    }
    groups.sort_unstable_by_key(|group| group.id);
    Ok(groups)
}

fn validate_distribution_group_pixels(
    shape: Shape,
    pixels_per_cell: u8,
    groups: &[PoolDistributionGroup],
) -> Result<(), EntityError> {
    for group in groups {
        for pixel in &group.pixels {
            if !pixel_is_in_footprint(shape, *pixel, pixels_per_cell) {
                return Err(EntityError::PoolDistributionPixelOutsideFootprint(*pixel));
            }
        }
    }
    Ok(())
}

fn validate_pool_boundary(boundary: PoolBoundary) -> Result<(), EntityError> {
    if boundary.padding_pixels.is_some_and(|value| value > 1024)
        || boundary
            .corner_radius_pixels
            .is_some_and(|value| value > 1024)
    {
        return Err(EntityError::InvalidPoolBoundary);
    }
    Ok(())
}

fn validate_collect_layers(layers: &[CollectLayer]) -> Result<(), EntityError> {
    if layers.len() > MAX_COLLECT_LAYERS {
        return Err(EntityError::CollectLayerLimit {
            actual: layers.len(),
        });
    }
    for layer in layers {
        let color_index = layer.color_index();
        if color_index != DISABLED_COLLECT_COLOR_INDEX && color_index > MAX_COLLECT_COLOR_INDEX {
            return Err(EntityError::InvalidCollectLayerColor(color_index));
        }
        if let CollectCapacity::Finite(value) = layer.capacity()
            && value > MAX_COLLECT_CAPACITY
        {
            return Err(EntityError::CollectCapacityTooLarge(value));
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

fn nearest_source_pixel(destination: u8, source_size: u8, destination_size: u8) -> u8 {
    let numerator = (2 * u16::from(destination) + 1) * u16::from(source_size);
    let denominator = 2 * u16::from(destination_size);
    (numerator / denominator) as u8
}

fn resampled_source_pixel(
    destination: BlindPixel,
    source_size: u8,
    destination_size: u8,
) -> BlindPixel {
    let destination_size = u16::from(destination_size);
    let source_size_u16 = u16::from(source_size);
    let cell_x = destination.x / destination_size;
    let cell_y = destination.y / destination_size;
    let local_x = (destination.x % destination_size) as u8;
    let local_y = (destination.y % destination_size) as u8;
    BlindPixel::new(
        cell_x * source_size_u16
            + u16::from(nearest_source_pixel(
                local_x,
                source_size,
                destination_size as u8,
            )),
        cell_y * source_size_u16
            + u16::from(nearest_source_pixel(
                local_y,
                source_size,
                destination_size as u8,
            )),
    )
}

fn pixel_is_in_footprint(shape: Shape, pixel: BlindPixel, pixels_per_cell: u8) -> bool {
    let resolution = u16::from(pixels_per_cell);
    let cell_x = pixel.x / resolution;
    let cell_y = pixel.y / resolution;
    cell_x < u16::from(shape.width())
        && cell_y < u16::from(shape.height())
        && shape.contains(ShapeCell::new(cell_x as u8, cell_y as u8))
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

    #[error(
        "Pool has {actual} explicit distribution groups, exceeding the {MAX_POOL_DISTRIBUTION_GROUPS}-group limit"
    )]
    PoolDistributionGroupLimit { actual: usize },

    #[error("distribution group ID zero is reserved for implicit Default")]
    ReservedPoolDistributionGroupId,

    #[error("distribution group ID {0} occurs more than once")]
    DuplicatePoolDistributionGroupId(u32),

    #[error(
        "distribution group {0} name must contain 1..={MAX_POOL_DISTRIBUTION_GROUP_NAME_CHARS} characters after trimming"
    )]
    InvalidPoolDistributionGroupName(u32),

    #[error("distribution group {id} contains more pixels than any Blind footprint")]
    PoolDistributionGroupPixelLimit { id: u32 },

    #[error("distribution pixel {0:?} occurs more than once within or across groups")]
    DuplicatePoolDistributionPixel(BlindPixel),

    #[error("distribution pixel {0:?} is outside the Blind footprint")]
    PoolDistributionPixelOutsideFootprint(BlindPixel),

    #[error("pool padding and corner radius must be at most 1024 pixels")]
    InvalidPoolBoundary,

    #[error("entity '{0}' is not a Blind")]
    NotBlind(EntityId),

    #[error("entity '{0}' is not a Block")]
    NotBlock(EntityId),

    #[error("entity '{entity_id}' has no collect layer at index {layer_index}")]
    CollectLayerNotFound {
        entity_id: EntityId,
        layer_index: usize,
    },

    #[error("entity '{entity_id}' collect layer {layer_index} is locked")]
    CollectLayerLocked {
        entity_id: EntityId,
        layer_index: usize,
    },

    #[error("Block has {actual} collect layers, exceeding the {MAX_COLLECT_LAYERS}-layer limit")]
    CollectLayerLimit { actual: usize },

    #[error("collect color index must be disabled or in 0..={MAX_COLLECT_COLOR_INDEX}, got {0}")]
    InvalidCollectLayerColor(u16),

    #[error("collect capacity exceeds the {MAX_COLLECT_CAPACITY} limit, got {0}")]
    CollectCapacityTooLarge(u32),

    #[error(transparent)]
    InvalidBrush(#[from] BrushError),
}
