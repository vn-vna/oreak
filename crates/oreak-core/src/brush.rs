use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::{BlindTile, ShapeCell};

pub const MIN_BLIND_COLOR_INDEX: u8 = 1;
pub const MAX_BLIND_COLOR_INDEX: u8 = 10;
pub const MAX_BLIND_BRUSH_PIXELS: usize = 64 * 32 * 32;
pub const MAX_BLIND_GUIDES: usize = MAX_BLIND_BRUSH_PIXELS * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlindPixel {
    pub x: u16,
    pub y: u16,
}

impl BlindPixel {
    #[must_use]
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct BlindGuide {
    first: BlindPixel,
    second: BlindPixel,
}

#[derive(Deserialize)]
struct SerializedBlindGuide {
    first: BlindPixel,
    second: BlindPixel,
}

impl<'de> Deserialize<'de> for BlindGuide {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let guide = SerializedBlindGuide::deserialize(deserializer)?;
        Self::new(guide.first, guide.second).map_err(D::Error::custom)
    }
}

impl BlindGuide {
    pub fn new(first: BlindPixel, second: BlindPixel) -> Result<Self, BrushError> {
        let distance =
            u32::from(first.x.abs_diff(second.x)) + u32::from(first.y.abs_diff(second.y));
        if distance != 1 {
            return Err(BrushError::GuideEndpointsNotAdjacent { first, second });
        }
        let (first, second) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        Ok(Self { first, second })
    }

    #[must_use]
    pub const fn first(self) -> BlindPixel {
        self.first
    }

    #[must_use]
    pub const fn second(self) -> BlindPixel {
        self.second
    }

    pub(crate) const fn from_transformed(first: BlindPixel, second: BlindPixel) -> Self {
        if first.x < second.x || (first.x == second.x && first.y <= second.y) {
            Self { first, second }
        } else {
            Self {
                first: second,
                second: first,
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BlindStroke {
    pixels: Vec<BlindPixel>,
}

#[derive(Deserialize)]
struct SerializedBlindStroke {
    pixels: Vec<BlindPixel>,
}

impl<'de> Deserialize<'de> for BlindStroke {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let stroke = SerializedBlindStroke::deserialize(deserializer)?;
        Self::new(stroke.pixels).map_err(D::Error::custom)
    }
}

impl BlindStroke {
    pub fn new(mut pixels: Vec<BlindPixel>) -> Result<Self, BrushError> {
        if pixels.len() > MAX_BLIND_BRUSH_PIXELS {
            return Err(BrushError::StrokeTooLarge {
                actual: pixels.len(),
            });
        }
        pixels.sort_unstable();
        pixels.dedup();
        Ok(Self { pixels })
    }

    #[must_use]
    pub fn pixels(&self) -> &[BlindPixel] {
        &self.pixels
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BlindGuideSet {
    guides: Vec<BlindGuide>,
}

#[derive(Deserialize)]
struct SerializedBlindGuideSet {
    guides: Vec<BlindGuide>,
}

impl<'de> Deserialize<'de> for BlindGuideSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let set = SerializedBlindGuideSet::deserialize(deserializer)?;
        Self::new(set.guides).map_err(D::Error::custom)
    }
}

impl BlindGuideSet {
    pub fn new(mut guides: Vec<BlindGuide>) -> Result<Self, BrushError> {
        if guides.len() > MAX_BLIND_GUIDES {
            return Err(BrushError::GuideSetTooLarge {
                actual: guides.len(),
            });
        }
        guides.sort_unstable();
        guides.dedup();
        Ok(Self { guides })
    }

    #[must_use]
    pub fn guides(&self) -> &[BlindGuide] {
        &self.guides
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.guides.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlindBrushOperation {
    PaintStroke {
        color_index: u8,
        stroke: BlindStroke,
    },
    EraseStroke {
        stroke: BlindStroke,
    },
    FloodFill {
        start: BlindPixel,
        color_index: u8,
    },
    AddGuides {
        guides: BlindGuideSet,
    },
    RemoveGuides {
        guides: BlindGuideSet,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlindTilePatch {
    pub cell: ShapeCell,
    pub tile: BlindTile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlindGuidePatch {
    pub guide: BlindGuide,
    pub present: bool,
}

pub(crate) fn validate_color_index(color_index: u8) -> Result<(), BrushError> {
    if !(MIN_BLIND_COLOR_INDEX..=MAX_BLIND_COLOR_INDEX).contains(&color_index) {
        return Err(BrushError::InvalidColorIndex(color_index));
    }
    Ok(())
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BrushError {
    #[error(
        "Blind color index must be in {MIN_BLIND_COLOR_INDEX}..={MAX_BLIND_COLOR_INDEX}, got {0}"
    )]
    InvalidColorIndex(u8),

    #[error("Blind stroke has {actual} pixels, exceeding the {MAX_BLIND_BRUSH_PIXELS}-pixel limit")]
    StrokeTooLarge { actual: usize },

    #[error("Blind guide set has {actual} edges, exceeding the {MAX_BLIND_GUIDES}-edge limit")]
    GuideSetTooLarge { actual: usize },

    #[error(
        "Blind guide endpoints ({}, {}) and ({}, {}) are not four-neighbor adjacent",
        first.x,
        first.y,
        second.x,
        second.y
    )]
    GuideEndpointsNotAdjacent {
        first: BlindPixel,
        second: BlindPixel,
    },

    #[error("Blind tile patch targets duplicate cell ({}, {})", .0.x, .0.y)]
    DuplicateTilePatch(ShapeCell),

    #[error("Blind tile patch set has {actual} entries, exceeding the 64-tile limit")]
    TilePatchSetTooLarge { actual: usize },

    #[error("Blind guide patch targets the same edge more than once")]
    DuplicateGuidePatch(BlindGuide),

    #[error("Blind tile patch targets unoccupied shape cell ({}, {})", .0.x, .0.y)]
    InvalidTilePatchCell(ShapeCell),

    #[error("Blind guide lies outside the occupied pixel footprint")]
    GuideOutsideFootprint(BlindGuide),
}
