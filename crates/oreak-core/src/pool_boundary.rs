//! Pixel-exact playable Pool geometry, matching the reference Unity runtime.
//! Padding erodes the whole footprint (never individual groups or tile seams).
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BlindPixel, PlaceableEntity, ShapeCell};

/// Absent values inherit the reference project's 0.04-cell defaults.
/// Explicit zero is a real override. Artwork is never erased by this metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolBoundary {
    pub padding_pixels: Option<u16>,
    pub corner_radius_pixels: Option<u16>,
}

impl PoolBoundary {
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.padding_pixels.is_none() && self.corner_radius_pixels.is_none()
    }

    #[must_use]
    pub fn resolved(self, pixels_per_cell: u8) -> (u16, u16) {
        // Unity stores the cell proportion as a float, then promotes to double.
        let inherited = (f64::from(pixels_per_cell) * f64::from(0.04_f32)).ceil() as u16;
        (
            self.padding_pixels.unwrap_or(inherited),
            self.corner_radius_pixels.unwrap_or(inherited),
        )
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PoolBoundaryError {
    #[error("boundary geometry requires a Pool")]
    NotPool,
    #[error("Pool corner radius {radius} exceeds maximum {maximum} at this resolution and padding")]
    CornerRadius { radius: u16, maximum: u16 },
    #[error(
        "Pool padding or rounded corners leave no playable pixels; reduce padding or increase resolution"
    )]
    NoPlayablePixels,
}

#[derive(Clone, Debug)]
pub struct PoolPlayableMask {
    pub width: u16,
    pub height: u16,
    pub padding_pixels: u16,
    pub corner_radius_pixels: u16,
    pixels: Vec<bool>,
}

impl PoolPlayableMask {
    #[must_use]
    pub fn contains(&self, pixel: BlindPixel) -> bool {
        pixel.x < self.width
            && pixel.y < self.height
            && self.pixels[usize::from(pixel.x) + usize::from(pixel.y) * usize::from(self.width)]
    }
}

/// Resolve geometry once per Pool, then intersect this mask with source groups.
/// Uses bounded eight-neighbour distance propagation for Chebyshev erosion.
pub fn pool_playable_mask(entity: &PlaceableEntity) -> Result<PoolPlayableMask, PoolBoundaryError> {
    let blind = entity.as_blind().ok_or(PoolBoundaryError::NotPool)?;
    let shape = entity.shape();
    let resolution = u16::from(blind.pixels_per_cell());
    let width = u16::from(shape.width()) * resolution;
    let height = u16::from(shape.height()) * resolution;
    let (padding, radius) = blind.boundary().resolved(blind.pixels_per_cell());
    let maximum_radius = resolution.saturating_sub(padding.saturating_mul(2)) / 2;
    if radius > maximum_radius {
        return Err(PoolBoundaryError::CornerRadius {
            radius,
            maximum: maximum_radius,
        });
    }
    let occupied = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && x < i32::from(shape.width())
            && y < i32::from(shape.height())
            && shape.contains(ShapeCell::new(x as u8, y as u8))
    };
    let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height));
    for y in 0..height {
        for x in 0..width {
            pixels.push(occupied(
                i32::from(x / resolution),
                i32::from(y / resolution),
            ));
        }
    }
    if padding > 0 {
        let mut distance = vec![u16::MAX; pixels.len()];
        let mut queue = VecDeque::new();
        for y in 0..height {
            for x in 0..width {
                let index = usize::from(x) + usize::from(y) * usize::from(width);
                if !pixels[index] {
                    distance[index] = 0;
                    queue.push_back((x, y));
                } else if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
                    distance[index] = 1;
                    queue.push_back((x, y));
                }
            }
        }
        while let Some((x, y)) = queue.pop_front() {
            let index = usize::from(x) + usize::from(y) * usize::from(width);
            let next = distance[index] + 1;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let nx = i32::from(x) + dx;
                    let ny = i32::from(y) + dy;
                    if nx < 0 || ny < 0 || nx >= i32::from(width) || ny >= i32::from(height) {
                        continue;
                    }
                    let neighbour = nx as usize + ny as usize * usize::from(width);
                    if distance[neighbour] > next {
                        distance[neighbour] = next;
                        queue.push_back((nx as u16, ny as u16));
                    }
                }
            }
        }
        for (pixel, distance) in pixels.iter_mut().zip(distance) {
            *pixel &= distance > padding;
        }
    }
    if !pixels.iter().any(|pixel| *pixel) {
        return Err(PoolBoundaryError::NoPlayablePixels);
    }
    if radius > 0 {
        let circle_squared = 4 * i64::from(radius) * i64::from(radius);
        for cell in shape.occupied_cells() {
            let cx = i32::from(cell.x);
            let cy = i32::from(cell.y);
            for (sx, sy) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                // A convex corner exists only when both cardinal neighbours are absent.
                if occupied(cx - sx, cy) || occupied(cx, cy - sy) {
                    continue;
                }
                let anchor_x = if sx > 0 {
                    cx * i32::from(resolution) + i32::from(padding)
                } else {
                    (cx + 1) * i32::from(resolution) - i32::from(padding) - 1
                };
                let anchor_y = if sy > 0 {
                    cy * i32::from(resolution) + i32::from(padding)
                } else {
                    (cy + 1) * i32::from(resolution) - i32::from(padding) - 1
                };
                for v in 0..radius {
                    for u in 0..radius {
                        let dx = 2 * i64::from(radius - u) - 1;
                        let dy = 2 * i64::from(radius - v) - 1;
                        if dx * dx + dy * dy <= circle_squared {
                            continue;
                        }
                        let x = anchor_x + sx * i32::from(u);
                        let y = anchor_y + sy * i32::from(v);
                        if x >= 0 && y >= 0 && x < i32::from(width) && y < i32::from(height) {
                            pixels[x as usize + y as usize * usize::from(width)] = false;
                        }
                    }
                }
            }
        }
    }
    if !pixels.iter().any(|pixel| *pixel) {
        return Err(PoolBoundaryError::NoPlayablePixels);
    }
    Ok(PoolPlayableMask {
        width,
        height,
        padding_pixels: padding,
        corner_radius_pixels: radius,
        pixels,
    })
}
