use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

pub const MAX_SHAPE_AREA: u16 = 64;
pub const MAX_BLOCK_SHAPE_AXIS: u8 = 8;

/// A coordinate inside a shape's lower-left-based bounding rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShapeCell {
    pub x: u8,
    pub y: u8,
}

impl ShapeCell {
    #[must_use]
    pub const fn new(x: u8, y: u8) -> Self {
        Self { x, y }
    }
}

/// A tightly bounded, four-neighbor-connected occupancy mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Shape {
    width: u8,
    height: u8,
    occupied_mask: u64,
}

#[derive(Deserialize)]
struct SerializedShape {
    width: u8,
    height: u8,
    occupied_mask: u64,
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let shape = SerializedShape::deserialize(deserializer)?;
        Self::new(shape.width, shape.height, shape.occupied_mask).map_err(D::Error::custom)
    }
}

impl Shape {
    pub fn new(width: u8, height: u8, occupied_mask: u64) -> Result<Self, ShapeError> {
        if width == 0 || height == 0 {
            return Err(ShapeError::InvalidDimensions { width, height });
        }

        let area = u16::from(width) * u16::from(height);
        if area > MAX_SHAPE_AREA {
            return Err(ShapeError::AreaTooLarge {
                width,
                height,
                area,
            });
        }

        let valid_mask = if area == MAX_SHAPE_AREA {
            u64::MAX
        } else {
            (1_u64 << area) - 1
        };
        if occupied_mask & !valid_mask != 0 {
            return Err(ShapeError::BitsOutsideBounds);
        }
        if occupied_mask == 0 {
            return Err(ShapeError::Empty);
        }

        let shape = Self {
            width,
            height,
            occupied_mask,
        };
        if !shape.is_tightly_bounded() {
            return Err(ShapeError::NotTightlyBounded);
        }
        if !shape.is_connected() {
            return Err(ShapeError::Disconnected);
        }

        Ok(shape)
    }

    pub fn from_cells(cells: &[ShapeCell]) -> Result<Self, ShapeError> {
        if cells.is_empty() {
            return Err(ShapeError::Empty);
        }

        let width = cells
            .iter()
            .map(|cell| cell.x)
            .max()
            .unwrap_or_default()
            .checked_add(1)
            .ok_or(ShapeError::AreaTooLarge {
                width: u8::MAX,
                height: 1,
                area: u16::MAX,
            })?;
        let height = cells
            .iter()
            .map(|cell| cell.y)
            .max()
            .unwrap_or_default()
            .checked_add(1)
            .ok_or(ShapeError::AreaTooLarge {
                width,
                height: u8::MAX,
                area: u16::MAX,
            })?;
        let area = u16::from(width) * u16::from(height);
        if area > MAX_SHAPE_AREA {
            return Err(ShapeError::AreaTooLarge {
                width,
                height,
                area,
            });
        }

        let mut occupied_mask = 0_u64;
        for cell in cells {
            let bit = u32::from(cell.x) + u32::from(cell.y) * u32::from(width);
            let cell_mask = 1_u64 << bit;
            if occupied_mask & cell_mask != 0 {
                return Err(ShapeError::DuplicateCell(*cell));
            }
            occupied_mask |= cell_mask;
        }

        Self::new(width, height, occupied_mask)
    }

    #[must_use]
    pub const fn width(self) -> u8 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u8 {
        self.height
    }

    #[must_use]
    pub const fn occupied_mask(self) -> u64 {
        self.occupied_mask
    }

    #[must_use]
    pub const fn bounding_area(self) -> u16 {
        self.width as u16 * self.height as u16
    }

    #[must_use]
    pub const fn occupied_count(self) -> u32 {
        self.occupied_mask.count_ones()
    }

    #[must_use]
    pub fn contains(self, cell: ShapeCell) -> bool {
        if cell.x >= self.width || cell.y >= self.height {
            return false;
        }

        self.occupied_mask & self.bit_for(cell.x, cell.y) != 0
    }

    pub fn occupied_cells(self) -> impl Iterator<Item = ShapeCell> {
        let width = self.width;
        (0..self.bounding_area())
            .filter(move |index| self.occupied_mask & (1_u64 << u32::from(*index)) != 0)
            .map(move |index| ShapeCell {
                x: (index % u16::from(width)) as u8,
                y: (index / u16::from(width)) as u8,
            })
    }

    #[must_use]
    pub fn rotated_clockwise(self) -> Self {
        let mut occupied_mask = 0_u64;
        for cell in self.occupied_cells() {
            let transformed = self.rotate_cell_clockwise(cell);
            let bit = u32::from(transformed.x) + u32::from(transformed.y) * u32::from(self.height);
            occupied_mask |= 1_u64 << bit;
        }

        Self {
            width: self.height,
            height: self.width,
            occupied_mask,
        }
    }

    #[must_use]
    pub fn flipped_horizontal(self) -> Self {
        let mut occupied_mask = 0_u64;
        for cell in self.occupied_cells() {
            let transformed = self.flip_cell_horizontal(cell);
            occupied_mask |= self.bit_for(transformed.x, transformed.y);
        }

        Self {
            occupied_mask,
            ..self
        }
    }

    #[must_use]
    pub(crate) const fn rotate_cell_clockwise(self, cell: ShapeCell) -> ShapeCell {
        ShapeCell::new(self.height - 1 - cell.y, cell.x)
    }

    #[must_use]
    pub(crate) const fn flip_cell_horizontal(self, cell: ShapeCell) -> ShapeCell {
        ShapeCell::new(self.width - 1 - cell.x, cell.y)
    }

    const fn bit_for(self, x: u8, y: u8) -> u64 {
        1_u64 << (x as u32 + y as u32 * self.width as u32)
    }

    fn is_tightly_bounded(self) -> bool {
        let mut touches_left = false;
        let mut touches_right = false;
        let mut touches_bottom = false;
        let mut touches_top = false;
        for cell in self.occupied_cells() {
            touches_left |= cell.x == 0;
            touches_right |= cell.x == self.width - 1;
            touches_bottom |= cell.y == 0;
            touches_top |= cell.y == self.height - 1;
        }

        touches_left && touches_right && touches_bottom && touches_top
    }

    fn is_connected(self) -> bool {
        let first = self.occupied_mask.trailing_zeros() as u8;
        let mut stack = [0_u8; MAX_SHAPE_AREA as usize];
        let mut stack_len = 1_usize;
        let mut visited = 1_u64 << first;
        stack[0] = first;

        while stack_len > 0 {
            stack_len -= 1;
            let index = stack[stack_len];
            let x = index % self.width;
            let y = index / self.width;
            let neighbors = [
                x.checked_sub(1).map(|next| (next, y)),
                x.checked_add(1)
                    .filter(|next| *next < self.width)
                    .map(|next| (next, y)),
                y.checked_sub(1).map(|next| (x, next)),
                y.checked_add(1)
                    .filter(|next| *next < self.height)
                    .map(|next| (x, next)),
            ];
            for (next_x, next_y) in neighbors.into_iter().flatten() {
                let next = next_x + next_y * self.width;
                let next_bit = 1_u64 << next;
                if self.occupied_mask & next_bit != 0 && visited & next_bit == 0 {
                    visited |= next_bit;
                    stack[stack_len] = next;
                    stack_len += 1;
                }
            }
        }

        visited == self.occupied_mask
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ShapeError {
    #[error("shape dimensions must be non-zero, got {width} x {height}")]
    InvalidDimensions { width: u8, height: u8 },

    #[error("shape bounding area {area} ({width} x {height}) exceeds {MAX_SHAPE_AREA}")]
    AreaTooLarge { width: u8, height: u8, area: u16 },

    #[error("shape is empty")]
    Empty,

    #[error("shape has occupied bits outside its bounding rectangle")]
    BitsOutsideBounds,

    #[error("shape has duplicate cell ({}, {})", .0.x, .0.y)]
    DuplicateCell(ShapeCell),

    #[error("shape bounding rectangle contains an empty outer edge")]
    NotTightlyBounded,

    #[error("shape cells are not connected by four-neighbor edges")]
    Disconnected,
}
