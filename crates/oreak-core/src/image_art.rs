//! Deterministic, bounded image quantization and projection onto existing Blinds.
//!
//! Images and placements use top-down coordinates. Blind tiles retain their
//! lower-left pixel coordinates; only the pixel row *within each cell* is flipped.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Blind, BlindTile, EntityError, PlaceableEntity};

pub const MAX_IMAGE_AXIS: u32 = 1024;
pub const MAX_PALETTE_MAPPINGS: usize = 32;
pub const PALETTE_VERSION: u32 = 1;
/// CSS colors corresponding to opaque palette indices 1..=10.
pub const PALETTE_HEX: [&str; 10] = [
    "#ff8b68", "#ffd15a", "#72dd91", "#54caec", "#7895ff", "#af82f2", "#f17bd2", "#ff7489",
    "#b69a7b", "#f4f7fa",
];
/// Fixed web `blind_color` palette. Index zero is transparent, not an RGB color.
pub const PALETTE_RGB: [[u8; 3]; 10] = [
    [0xff, 0x8b, 0x68],
    [0xff, 0xd1, 0x5a],
    [0x72, 0xdd, 0x91],
    [0x54, 0xca, 0xec],
    [0x78, 0x95, 0xff],
    [0xaf, 0x82, 0xf2],
    [0xf1, 0x7b, 0xd2],
    [0xff, 0x74, 0x89],
    [0xb6, 0x9a, 0x7b],
    [0xf4, 0xf7, 0xfa],
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DitherMode {
    #[default]
    None,
    FloydSteinberg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteMapping {
    pub source_rgb: [u8; 3],
    pub target_color: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteSettings {
    /// A nonempty set of opaque palette indices (1..=10). Order is immaterial.
    pub enabled_colors: Vec<u8>,
    pub dithering: DitherMode,
    /// Alpha below this value becomes transparent; zero alpha is always transparent.
    pub alpha_threshold: u8,
    /// Optional enabled opaque color used to flatten RGBA before quantization.
    pub background: Option<u8>,
    /// Optional source-color anchors mapped onto enabled palette colors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mappings: Vec<PaletteMapping>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceColorCluster {
    pub rgb: [u8; 3],
    pub pixel_count: u32,
}

impl Default for PaletteSettings {
    fn default() -> Self {
        Self {
            enabled_colors: (1..=10).collect(),
            dithering: DitherMode::None,
            alpha_threshold: 128,
            background: None,
            mappings: Vec::new(),
        }
    }
}

impl PaletteSettings {
    pub fn validate(&self) -> Result<(), ImageArtError> {
        if self.enabled_colors.is_empty() {
            return Err(ImageArtError::EmptyPalette);
        }
        let mut seen = [false; 11];
        for &color in &self.enabled_colors {
            if !(1..=10).contains(&color) {
                return Err(ImageArtError::InvalidColor(color));
            }
            if seen[usize::from(color)] {
                return Err(ImageArtError::DuplicateColor(color));
            }
            seen[usize::from(color)] = true;
        }
        if let Some(background) = self.background
            && (!(1..=10).contains(&background) || !seen[usize::from(background)])
        {
            return Err(ImageArtError::InvalidBackground(background));
        }
        if self.mappings.len() > MAX_PALETTE_MAPPINGS {
            return Err(ImageArtError::TooManyMappings(self.mappings.len()));
        }
        for (index, mapping) in self.mappings.iter().enumerate() {
            if !(1..=10).contains(&mapping.target_color) || !seen[usize::from(mapping.target_color)]
            {
                return Err(ImageArtError::InvalidMappingTarget(mapping.target_color));
            }
            if self.mappings[..index]
                .iter()
                .any(|candidate| color_bin(candidate.source_rgb) == color_bin(mapping.source_rgb))
            {
                return Err(ImageArtError::DuplicateMappingSource(mapping.source_rgb));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    /// Row-major, top-down palette indices; zero means transparent.
    pub pixels: Vec<u8>,
    pub palette_version: u32,
}

impl IndexedImage {
    pub fn validate(&self) -> Result<(), ImageArtError> {
        let expected = pixel_count(self.width, self.height)?;
        if self.palette_version != PALETTE_VERSION {
            return Err(ImageArtError::UnsupportedPaletteVersion(
                self.palette_version,
            ));
        }
        if self.pixels.len() != expected {
            return Err(ImageArtError::PixelCount {
                expected,
                actual: self.pixels.len(),
            });
        }
        if let Some(&color) = self.pixels.iter().find(|&&color| color > 10) {
            return Err(ImageArtError::InvalidColor(color));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageSampling {
    #[default]
    Nearest,
    Area,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageTransparency {
    #[default]
    Preserve,
    Erase,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImagePlacement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub sampling: ImageSampling,
    /// Source-pixel block side, 1..=1024; each block uses its top-left pixel.
    pub pixelation: u16,
    /// Resample the whole existing Blind before projecting, if supplied.
    pub resolution: Option<u8>,
    pub transparency: ImageTransparency,
}

// Total float equality remains reflexive for invalid values assembled through
// public fields, while validation rejects them before application. Signed zeros
// remain distinct so equality is consistent with the serialized representation.
impl PartialEq for ImagePlacement {
    fn eq(&self, other: &Self) -> bool {
        self.x.total_cmp(&other.x).is_eq()
            && self.y.total_cmp(&other.y).is_eq()
            && self.width.total_cmp(&other.width).is_eq()
            && self.height.total_cmp(&other.height).is_eq()
            && self.sampling == other.sampling
            && self.pixelation == other.pixelation
            && self.resolution == other.resolution
            && self.transparency == other.transparency
    }
}

impl Eq for ImagePlacement {}

impl<'de> Deserialize<'de> for ImagePlacement {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            x: f64,
            y: f64,
            width: f64,
            height: f64,
            sampling: ImageSampling,
            pixelation: u16,
            resolution: Option<u8>,
            transparency: ImageTransparency,
        }
        let fields = Fields::deserialize(deserializer)?;
        let value = Self {
            x: fields.x,
            y: fields.y,
            width: fields.width,
            height: fields.height,
            sampling: fields.sampling,
            pixelation: fields.pixelation,
            resolution: fields.resolution,
            transparency: fields.transparency,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl ImagePlacement {
    pub fn validate(&self) -> Result<(), ImageArtError> {
        if ![self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            || self.width <= 0.0
            || self.height <= 0.0
            || !(self.x + self.width).is_finite()
            || !(self.y + self.height).is_finite()
            || self.x + self.width <= self.x
            || self.y + self.height <= self.y
        {
            return Err(ImageArtError::InvalidPlacement);
        }
        if self.pixelation == 0 || u32::from(self.pixelation) > MAX_IMAGE_AXIS {
            return Err(ImageArtError::InvalidPixelation(self.pixelation));
        }
        if let Some(resolution) = self.resolution
            && !(1..=32).contains(&resolution)
        {
            return Err(ImageArtError::InvalidResolution(resolution));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ImageArtError {
    #[error("image dimensions {width} x {height} must be in 1..={MAX_IMAGE_AXIS}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("RGBA buffer contains {actual} bytes; expected {expected}")]
    RgbaLength { expected: usize, actual: usize },
    #[error("indexed image contains {actual} pixels; expected {expected}")]
    PixelCount { expected: usize, actual: usize },
    #[error("at least one opaque palette color must be enabled")]
    EmptyPalette,
    #[error("invalid palette index {0}")]
    InvalidColor(u8),
    #[error("duplicate enabled palette index {0}")]
    DuplicateColor(u8),
    #[error("background index {0} must be an enabled opaque palette color")]
    InvalidBackground(u8),
    #[error("palette mappings contain {0} entries; maximum is {MAX_PALETTE_MAPPINGS}")]
    TooManyMappings(usize),
    #[error("duplicate palette mapping source RGB {0:?}")]
    DuplicateMappingSource([u8; 3]),
    #[error("palette mapping target {0} must be an enabled opaque palette color")]
    InvalidMappingTarget(u8),
    #[error("maximum color clusters must be in 1..={MAX_PALETTE_MAPPINGS}, got {0}")]
    InvalidMaxClusters(usize),
    #[error("unsupported palette version {0}")]
    UnsupportedPaletteVersion(u32),
    #[error("placement coordinates must be finite with positive, representable dimensions")]
    InvalidPlacement,
    #[error("source pixelation must be in 1..=1024, got {0}")]
    InvalidPixelation(u16),
    #[error("Blind resolution must be in 1..=32, got {0}")]
    InvalidResolution(u8),
    #[error("placement sampling and pixelation must match the prepared image projector")]
    ProjectorSettingsMismatch,
    #[error(transparent)]
    Entity(#[from] EntityError),
}

/// Extract dominant opaque source colors using a bounded 5-bit-per-channel
/// histogram. Pixels with zero alpha are ignored. Within each histogram bin,
/// the returned RGB is the rounded average of the original 8-bit samples.
pub fn extract_color_clusters(
    width: u32,
    height: u32,
    rgba: &[u8],
    max_clusters: usize,
) -> Result<Vec<SourceColorCluster>, ImageArtError> {
    let count = pixel_count(width, height)?;
    let expected = count * 4;
    if rgba.len() != expected {
        return Err(ImageArtError::RgbaLength {
            expected,
            actual: rgba.len(),
        });
    }
    if !(1..=MAX_PALETTE_MAPPINGS).contains(&max_clusters) {
        return Err(ImageArtError::InvalidMaxClusters(max_clusters));
    }

    // 32^3 fixed bins keep memory and work bounded at the maximum image size.
    let mut histogram = vec![[0_u64; 4]; 32 * 32 * 32];
    for pixel in rgba.chunks_exact(4) {
        if pixel[3] == 0 {
            continue;
        }
        let bin = color_bin([pixel[0], pixel[1], pixel[2]]);
        for channel in 0..3 {
            histogram[bin][channel] += u64::from(pixel[channel]);
        }
        histogram[bin][3] += 1;
    }

    let mut clusters: Vec<_> = histogram
        .into_iter()
        .filter_map(|bin| {
            let count = bin[3];
            (count != 0).then(|| SourceColorCluster {
                rgb: std::array::from_fn(|channel| ((bin[channel] + count / 2) / count) as u8),
                pixel_count: count as u32,
            })
        })
        .collect();
    clusters.sort_unstable_by(|a, b| {
        b.pixel_count
            .cmp(&a.pixel_count)
            .then_with(|| a.rgb.cmp(&b.rgb))
    });
    clusters.truncate(max_clusters);
    Ok(clusters)
}

/// Quantize straight-alpha sRGB without resizing. Transparent pixels neither
/// receive nor distribute diffusion error. With a background, composite in
/// linear RGB (below-threshold alpha is treated as zero). Mappings override the
/// matching original 5-bit source histogram bucket before diffusion, independent
/// of background composition. Transparent pixels cannot be mapped. Unmapped
/// buckets retain normal nearest-enabled Oklab quantization. Diffusion is always
/// measured against the emitted target.
pub fn convert_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
    settings: &PaletteSettings,
) -> Result<IndexedImage, ImageArtError> {
    settings.validate()?;
    let count = pixel_count(width, height)?;
    let expected = count * 4;
    if rgba.len() != expected {
        return Err(ImageArtError::RgbaLength {
            expected,
            actual: rgba.len(),
        });
    }
    let quantizer = Quantizer::new(settings);
    let width_usize = width as usize;
    let mut pixels = vec![0; count];
    // Two scan lines bound diffusion storage even at maximum image size.
    let mut current = vec![[0.0; 3]; width_usize + 2];
    let mut next = current.clone();
    for y in 0..height as usize {
        for x in 0..width_usize {
            let index = y * width_usize + x;
            let pixel = &rgba[index * 4..index * 4 + 4];
            let transparent = pixel[3] == 0 || pixel[3] < settings.alpha_threshold;
            if transparent && settings.background.is_none() {
                continue;
            }
            let mut rgb = [linear(pixel[0]), linear(pixel[1]), linear(pixel[2])];
            if let Some(background) = settings.background {
                let alpha = if transparent {
                    0.0
                } else {
                    f64::from(pixel[3]) / 255.0
                };
                for (channel, value) in rgb.iter_mut().enumerate() {
                    *value = *value * alpha
                        + quantizer.linear[usize::from(background)][channel] * (1.0 - alpha);
                }
            }
            // Clusters are picked from the original, not the composited preview.
            // Transparent pixels belong to the background, never a source group.
            let mapped_color = if transparent {
                None
            } else {
                quantizer.mapping_target([pixel[0], pixel[1], pixel[2]])
            };
            if settings.dithering == DitherMode::FloydSteinberg {
                for (channel, value) in rgb.iter_mut().enumerate() {
                    *value = (*value + current[x + 1][channel]).clamp(0.0, 1.0);
                }
            }
            let color = mapped_color.unwrap_or_else(|| quantizer.nearest_linear(rgb));
            pixels[index] = color;
            if settings.dithering == DitherMode::FloydSteinberg {
                for (channel, value) in rgb.iter().enumerate() {
                    let error = *value - quantizer.linear[usize::from(color)][channel];
                    current[x + 2][channel] += error * (7.0 / 16.0);
                    next[x][channel] += error * (3.0 / 16.0);
                    next[x + 1][channel] += error * (5.0 / 16.0);
                    next[x + 2][channel] += error * (1.0 / 16.0);
                }
            }
        }
        std::mem::swap(&mut current, &mut next);
        next.fill([0.0; 3]);
    }
    Ok(IndexedImage {
        width,
        height,
        pixels,
        palette_version: PALETTE_VERSION,
    })
}

/// Project into occupied destination pixel centers in the half-open placement
/// rectangle. Area sampling averages palette sRGB with transparent coverage,
/// then requantizes to the enabled palette. It uses an exact summed-area table:
/// O(source pixels + destination pixels), never a source scan per destination.
/// Dithering belongs to conversion, not projection. Guides are preserved, or
/// resampled using Blind's existing guide semantics when resolution changes.
pub fn project_image(
    image: &IndexedImage,
    settings: &PaletteSettings,
    placement: &ImagePlacement,
    entity: &PlaceableEntity,
) -> Result<PlaceableEntity, ImageArtError> {
    placement.validate()?;
    ImageProjector::new(image, settings, placement.sampling, placement.pixelation)?
        .project(placement, entity)
}

/// Reusable immutable image projection state. Construction validates and copies
/// the source once and builds at most one summed-area table. Retain this context
/// across target entities and placement changes: `project` performs no source
/// scans, source copies, or summed-area allocations. It only processes the
/// destination Blind (at most 64 * 32 * 32 pixels).
///
/// Rebuild when image, palette settings, sampling, or pixelation changes. Use
/// `Rc`/`Arc` when sharing a context; cloning its large prepared buffer is avoided.
#[derive(Debug)]
pub struct ImageProjector {
    image: IndexedImage,
    settings: PaletteSettings,
    sampling: ImageSampling,
    pixelation: u16,
    quantizer: Quantizer,
    remap: [u8; 11],
    area: Option<AreaTable>,
}

impl ImageProjector {
    pub fn new(
        image: &IndexedImage,
        settings: &PaletteSettings,
        sampling: ImageSampling,
        pixelation: u16,
    ) -> Result<Self, ImageArtError> {
        image.validate()?;
        settings.validate()?;
        if pixelation == 0 || u32::from(pixelation) > MAX_IMAGE_AXIS {
            return Err(ImageArtError::InvalidPixelation(pixelation));
        }
        let quantizer = Quantizer::new(settings);
        let mut remap = [0; 11];
        remap[0] = settings.background.unwrap_or(0);
        for (color, mapped) in remap.iter_mut().enumerate().skip(1) {
            *mapped = quantizer.nearest_linear(quantizer.linear[color]);
        }
        let area = (sampling == ImageSampling::Area).then(|| AreaTable::new(image, pixelation));
        Ok(Self {
            image: image.clone(),
            settings: settings.clone(),
            sampling,
            pixelation,
            quantizer,
            remap,
            area,
        })
    }

    /// Exact cache-key comparison; this may compare the whole source image.
    /// Call once per preview operation, not once per destination entity. Moving,
    /// resizing, changing resolution, or changing transparency needs no rebuild.
    #[must_use]
    pub fn matches_source(
        &self,
        image: &IndexedImage,
        settings: &PaletteSettings,
        sampling: ImageSampling,
        pixelation: u16,
    ) -> bool {
        self.sampling == sampling
            && self.pixelation == pixelation
            && self.settings == *settings
            && self.image == *image
    }

    pub fn project(
        &self,
        placement: &ImagePlacement,
        entity: &PlaceableEntity,
    ) -> Result<PlaceableEntity, ImageArtError> {
        placement.validate()?;
        if placement.sampling != self.sampling || placement.pixelation != self.pixelation {
            return Err(ImageArtError::ProjectorSettingsMismatch);
        }
        let image = &self.image;
        let settings = &self.settings;
        let quantizer = &self.quantizer;
        let remap = &self.remap;
        let area = &self.area;
        let blind = entity
            .as_blind()
            .ok_or_else(|| EntityError::NotBlind(entity.id().clone()))?;
        let resolution = placement.resolution.unwrap_or(blind.pixels_per_cell());
        let blind = blind.resampled(entity.shape(), resolution)?;
        let mut tiles = Vec::with_capacity(blind.tiles().len());
        let r = f64::from(resolution);
        for (cell, tile) in entity.shape().occupied_cells().zip(blind.tiles()) {
            let mut colors = tile.colors().to_vec();
            for py in 0..resolution {
                let gy = f64::from(entity.origin().y) + f64::from(cell.y) + 1.0
                    - (f64::from(py) + 0.5) / r;
                if gy < placement.y || gy >= placement.y + placement.height {
                    continue;
                }
                for px in 0..resolution {
                    let gx = f64::from(entity.origin().x)
                        + f64::from(cell.x)
                        + (f64::from(px) + 0.5) / r;
                    if gx < placement.x || gx >= placement.x + placement.width {
                        continue;
                    }
                    let sx = ((gx - placement.x) / placement.width) * f64::from(image.width);
                    let sy = ((gy - placement.y) / placement.height) * f64::from(image.height);
                    let nearest = || {
                        remap[usize::from(source_pixel(
                            image,
                            sx as u32,
                            sy as u32,
                            placement.pixelation,
                        ))]
                    };
                    let color = if let Some(area) = &area {
                        let half = 0.5 / r;
                        let x0 = ((gx - half - placement.x) / placement.width).clamp(0.0, 1.0)
                            * f64::from(image.width);
                        let x1 = ((gx + half - placement.x) / placement.width).clamp(0.0, 1.0)
                            * f64::from(image.width);
                        let y0 = ((gy - half - placement.y) / placement.height).clamp(0.0, 1.0)
                            * f64::from(image.height);
                        let y1 = ((gy + half - placement.y) / placement.height).clamp(0.0, 1.0)
                            * f64::from(image.height);
                        if x1 > x0 && y1 > y0 {
                            area.sample([x0, y0, x1, y1], settings, quantizer)
                        } else {
                            // A representable center can have an unrepresentably small box.
                            nearest()
                        }
                    } else {
                        nearest()
                    };
                    if color != 0 || placement.transparency == ImageTransparency::Erase {
                        colors[usize::from(py) * usize::from(resolution) + usize::from(px)] = color;
                    }
                }
            }
            tiles.push(BlindTile::from_colors(resolution, colors)?);
        }
        let next = Blind::with_guides(resolution, tiles, blind.guides().to_vec())?
            .with_distribution_groups(entity.shape(), blind.distribution_groups().to_vec())?
            .with_boundary(blind.boundary())?;
        Ok(PlaceableEntity::blind(
            entity.id().clone(),
            entity.origin(),
            entity.shape(),
            next,
        )?)
    }
}

fn pixel_count(width: u32, height: u32) -> Result<usize, ImageArtError> {
    if width == 0 || height == 0 || width > MAX_IMAGE_AXIS || height > MAX_IMAGE_AXIS {
        return Err(ImageArtError::InvalidDimensions { width, height });
    }
    Ok(width as usize * height as usize)
}

fn source_pixel(image: &IndexedImage, x: u32, y: u32, block: u16) -> u8 {
    let block = u32::from(block);
    let x = x.min(image.width - 1) / block * block;
    let y = y.min(image.height - 1) / block * block;
    image.pixels[(y * image.width + x) as usize]
}

fn color_bin(rgb: [u8; 3]) -> usize {
    (usize::from(rgb[0] >> 3) << 10) | (usize::from(rgb[1] >> 3) << 5) | usize::from(rgb[2] >> 3)
}

fn linear(value: u8) -> f64 {
    linear_unit(f64::from(value) / 255.0)
}

fn linear_unit(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn oklab(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb;
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

#[derive(Debug)]
struct Quantizer {
    enabled: [bool; 11],
    linear: [[f64; 3]; 11],
    labs: [[f64; 3]; 11],
    mapping_targets: Vec<u8>,
}

impl Quantizer {
    fn new(settings: &PaletteSettings) -> Self {
        let mut mapping_targets = vec![0; 32 * 32 * 32];
        for mapping in &settings.mappings {
            mapping_targets[color_bin(mapping.source_rgb)] = mapping.target_color;
        }
        let mut result = Self {
            enabled: [false; 11],
            linear: [[0.0; 3]; 11],
            labs: [[0.0; 3]; 11],
            mapping_targets,
        };
        for &color in &settings.enabled_colors {
            result.enabled[usize::from(color)] = true;
        }
        for (index, rgb) in PALETTE_RGB.iter().enumerate() {
            result.linear[index + 1] = rgb.map(linear);
            result.labs[index + 1] = oklab(result.linear[index + 1]);
        }
        result
    }

    fn mapping_target(&self, rgb: [u8; 3]) -> Option<u8> {
        let target = self.mapping_targets[color_bin(rgb)];
        (target != 0).then_some(target)
    }

    fn nearest_linear(&self, rgb: [f64; 3]) -> u8 {
        let lab = oklab(rgb);
        let mut best = 0;
        let mut distance = f64::INFINITY;
        // Ascending indices make exact ties independent of enabled-colors order.
        for color in 1..=10 {
            if !self.enabled[color] {
                continue;
            }
            let candidate = lab
                .iter()
                .zip(self.labs[color])
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f64>();
            if candidate < distance {
                best = color as u8;
                distance = candidate;
            }
        }
        best
    }
}

// At most three integer intervals, each weighted by its normalized per-pixel
// contribution. Normalizing axes separately also prevents tiny-area underflow.
fn weighted_ranges(start: f64, end: f64) -> [(usize, usize, f64); 3] {
    let first = start.floor() as usize;
    let last = end.ceil() as usize - 1;
    if first == last {
        return [(first, first + 1, 1.0), (0, 0, 0.0), (0, 0, 0.0)];
    }
    let length = end - start;
    [
        (first, first + 1, (first as f64 + 1.0 - start) / length),
        (
            first + 1,
            last,
            if last > first + 1 { 1.0 / length } else { 0.0 },
        ),
        (last, last + 1, (end - last as f64) / length),
    ]
}

/// Integer prefix sums avoid cumulative floating point error and use at most
/// 16*(1025^2) bytes. Channels hold sRGB sums and opaque-pixel counts.
#[derive(Debug)]
struct AreaTable {
    width: usize,
    sums: Vec<[u32; 4]>,
}

impl AreaTable {
    fn new(image: &IndexedImage, block: u16) -> Self {
        let width = image.width as usize;
        let height = image.height as usize;
        let mut sums = vec![[0; 4]; (width + 1) * (height + 1)];
        for y in 0..height {
            let mut row = [0; 4];
            for x in 0..width {
                let color = source_pixel(image, x as u32, y as u32, block);
                if color != 0 {
                    let rgb = PALETTE_RGB[usize::from(color) - 1];
                    for channel in 0..3 {
                        row[channel] += u32::from(rgb[channel]);
                    }
                    row[3] += 1;
                }
                let above = sums[y * (width + 1) + x + 1];
                sums[(y + 1) * (width + 1) + x + 1] = std::array::from_fn(|c| row[c] + above[c]);
            }
        }
        Self { width, sums }
    }

    fn rectangle(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> [u32; 4] {
        let at = |x, y| self.sums[y * (self.width + 1) + x];
        let a = at(x0, y0);
        let b = at(x1, y0);
        let c = at(x0, y1);
        let d = at(x1, y1);
        // Even two maximum prefix sums fit in u32 at the source-size limit.
        std::array::from_fn(|k| (d[k] + a[k]) - (b[k] + c[k]))
    }

    fn sample(&self, bounds: [f64; 4], settings: &PaletteSettings, quantizer: &Quantizer) -> u8 {
        let [x0, y0, x1, y1] = bounds;
        let mut sum = [0.0_f64; 4];
        // Split fractional boundary pixels from integer interior rectangles.
        // Unlike subtracting interpolated prefixes, this does not catastrophically
        // cancel for tiny source footprints at very large placement scales.
        for (left, right, wx) in weighted_ranges(x0, x1) {
            if wx == 0.0 {
                continue;
            }
            for (top, bottom, wy) in weighted_ranges(y0, y1) {
                if wy == 0.0 {
                    continue;
                }
                let rectangle = self.rectangle(left, top, right, bottom);
                for channel in 0..4 {
                    sum[channel] += f64::from(rectangle[channel]) * wx * wy;
                }
            }
        }
        let coverage = sum[3].clamp(0.0, 1.0);
        if coverage == 0.0 || coverage * 255.0 + 1e-9 < f64::from(settings.alpha_threshold) {
            return settings.background.unwrap_or(0);
        }
        let rgb = std::array::from_fn(|channel| {
            let average = (sum[channel] / sum[3] / 255.0).clamp(0.0, 1.0);
            if let Some(background) = settings.background {
                // Area sampling explicitly averages palette RGB, not palette indices.
                let bg = f64::from(PALETTE_RGB[usize::from(background) - 1][channel]) / 255.0;
                linear_unit(average * coverage + bg * (1.0 - coverage))
            } else {
                linear_unit(average)
            }
        });
        quantizer.nearest_linear(rgb)
    }
}
