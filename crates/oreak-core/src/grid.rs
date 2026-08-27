use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_GRID_AXIS: u16 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GridPoint {
    pub x: u16,
    pub y: u16,
}

impl GridPoint {
    #[must_use]
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridSize {
    width: u16,
    height: u16,
}

impl GridSize {
    pub fn new(width: u16, height: u16) -> Result<Self, GridError> {
        let size = Self { width, height };
        size.validate()?;
        Ok(size)
    }

    #[must_use]
    pub const fn width(self) -> u16 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u16 {
        self.height
    }

    #[must_use]
    pub fn cell_count(self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }

    pub fn validate(self) -> Result<(), GridError> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_GRID_AXIS
            || self.height > MAX_GRID_AXIS
        {
            return Err(GridError::InvalidSize {
                width: self.width,
                height: self.height,
            });
        }

        Ok(())
    }

    pub fn index_of(self, point: GridPoint) -> Result<usize, GridError> {
        if point.x >= self.width || point.y >= self.height {
            return Err(GridError::PointOutOfBounds {
                point,
                width: self.width,
                height: self.height,
            });
        }

        Ok(usize::from(point.x) + usize::from(point.y) * usize::from(self.width))
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GridError {
    #[error("grid size {width} x {height} is outside the supported 1..={MAX_GRID_AXIS} range")]
    InvalidSize { width: u16, height: u16 },

    #[error("grid point ({}, {}) is outside the {width} x {height} grid", point.x, point.y)]
    PointOutOfBounds {
        point: GridPoint,
        width: u16,
        height: u16,
    },

    #[error("grid contains {actual} cells but {expected} were expected")]
    CellCountMismatch { expected: usize, actual: usize },

    #[error("invalid level snapshot: {reason}")]
    InvalidSnapshot { reason: String },
}
