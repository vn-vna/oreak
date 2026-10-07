//! Pure, color-preserving allocation of selected Pool sand to collect layers.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    BlindPixel, CollectCapacity, EntityId, LevelSnapshot, MAX_COLLECT_CAPACITY,
    MAX_COLLECT_COLOR_INDEX, PlaceableEntity, PlaceableEntityKind,
};

pub const MAX_DISTRIBUTION_POOLS: usize = 64;
pub const MAX_DISTRIBUTION_LAYERS: usize = 256;
pub const MAX_DISTRIBUTION_WEIGHT: u32 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionRequest {
    pub pools: Vec<DistributionPoolSelection>,
    pub layers: Vec<DistributionLayerSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionPoolSelection {
    pub entity_id: EntityId,
    pub group_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionLayerSelection {
    pub entity_id: EntityId,
    pub layer_index: usize,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionPlan {
    /// Ascending color order, including source-only and target-only colors.
    pub colors: Vec<DistributionColorSummary>,
    /// Stable entity ID / layer index order, independent of request order.
    pub layers: Vec<DistributionLayerAllocation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionColorSummary {
    pub color_index: u16,
    /// Nonzero painted pixels, including non-playable border pixels.
    pub painted: u64,
    /// Painted pixels admitted by the shared Pool boundary policy.
    pub effective: u64,
    /// Finite capacities reserved by selected locked layers.
    pub locked: u64,
    /// Total final capacity, including locked reservations.
    pub allocated: u64,
    /// Effective source pixels with no selected target of this color.
    pub unmatched: u64,
}

impl DistributionColorSummary {
    fn empty(color_index: u16) -> Self {
        Self {
            color_index,
            painted: 0,
            effective: 0,
            locked: 0,
            allocated: 0,
            unmatched: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionLayerAllocation {
    pub entity_id: EntityId,
    pub layer_index: usize,
    pub before: CollectCapacity,
    pub capacity: u32,
    pub weight: u32,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum DistributionError {
    #[error("invalid boundary for Pool '{entity_id}': {reason}")]
    InvalidPoolBoundary { entity_id: EntityId, reason: String },
    #[error("select at least one source Pool")]
    EmptyPools,
    #[error("select at least one target layer")]
    EmptyLayers,
    #[error("selected {actual} Pools; maximum is {MAX_DISTRIBUTION_POOLS}")]
    TooManyPools { actual: usize },
    #[error("selected {actual} layers; maximum is {MAX_DISTRIBUTION_LAYERS}")]
    TooManyLayers { actual: usize },
    #[error("Pool '{0}' is selected more than once")]
    DuplicatePool(EntityId),
    #[error("entity '{0}' does not exist")]
    EntityNotFound(EntityId),
    #[error("entity '{0}' is not a Pool")]
    NotPool(EntityId),
    #[error("entity '{0}' is not a Block")]
    NotBlock(EntityId),
    #[error("select at least one group in Pool '{0}'")]
    EmptyGroups(EntityId),
    #[error("group {group_id} in Pool '{entity_id}' is selected more than once")]
    DuplicateGroup { entity_id: EntityId, group_id: u32 },
    #[error("Pool '{entity_id}' has no group {group_id}")]
    GroupNotFound { entity_id: EntityId, group_id: u32 },
    #[error("layer {layer_index} in '{entity_id}' is selected more than once")]
    DuplicateLayer {
        entity_id: EntityId,
        layer_index: usize,
    },
    #[error("entity '{entity_id}' has no layer {layer_index}")]
    LayerNotFound {
        entity_id: EntityId,
        layer_index: usize,
    },
    #[error("layer {layer_index} in '{entity_id}' has unsupported or disabled color {color_index}")]
    InvalidTargetColor {
        entity_id: EntityId,
        layer_index: usize,
        color_index: u16,
    },
    #[error(
        "layer {layer_index} in '{entity_id}' has weight {weight}, exceeding {MAX_DISTRIBUTION_WEIGHT}"
    )]
    WeightTooLarge {
        entity_id: EntityId,
        layer_index: usize,
        weight: u32,
    },
    #[error("locked layer {layer_index} in '{entity_id}' has unlimited capacity")]
    LockedUnlimited {
        entity_id: EntityId,
        layer_index: usize,
    },
    #[error("color {color_index} reserves {locked} locked pixels but has budget {effective}")]
    LockedExceedsBudget {
        color_index: u16,
        locked: u64,
        effective: u64,
    },
    #[error("color {color_index} has remaining budget {remaining} but zero unlocked weight")]
    ZeroTotalWeight { color_index: u16, remaining: u64 },
    #[error(
        "layer {layer_index} in '{entity_id}' would have capacity {capacity}, exceeding {MAX_COLLECT_CAPACITY}"
    )]
    CapacityOverflow {
        entity_id: EntityId,
        layer_index: usize,
        capacity: u64,
    },
}

/// Counts one group's nonzero pixels using precisely the planner's source counter.
/// Group zero is the footprint minus *all* explicit group memberships. Allocation,
/// lock and unmatched fields are zero because no target selection is supplied.
pub fn pool_group_counts(
    entity: &PlaceableEntity,
    group_id: u32,
) -> Result<Vec<DistributionColorSummary>, DistributionError> {
    Ok(count_pool_groups(entity, &[group_id])?
        .into_values()
        .collect())
}

fn count_pool_groups(
    entity: &PlaceableEntity,
    group_ids: &[u32],
) -> Result<BTreeMap<u16, DistributionColorSummary>, DistributionError> {
    let blind = entity
        .as_blind()
        .ok_or_else(|| DistributionError::NotPool(entity.id().clone()))?;
    if group_ids.is_empty() {
        return Err(DistributionError::EmptyGroups(entity.id().clone()));
    }
    let mut selected = BTreeSet::new();
    for &group_id in group_ids {
        if !selected.insert(group_id) {
            return Err(DistributionError::DuplicateGroup {
                entity_id: entity.id().clone(),
                group_id,
            });
        }
        if group_id != 0
            && !blind
                .distribution_groups()
                .iter()
                .any(|group| group.id == group_id)
        {
            return Err(DistributionError::GroupNotFound {
                entity_id: entity.id().clone(),
                group_id,
            });
        }
    }
    let mut colors = BTreeMap::new();
    for (group_id, counts) in pool_all_group_counts(entity)? {
        if !selected.contains(&group_id) {
            continue;
        }
        for count in counts {
            let summary = colors
                .entry(count.color_index)
                .or_insert_with(|| DistributionColorSummary::empty(count.color_index));
            summary.painted += count.painted;
            summary.effective += count.effective;
        }
    }
    Ok(colors)
}

/// Counts every group in one pass for a Pool inspector or other bulk consumer.
/// Includes Default zero and every explicit group ID, even when its counts are
/// empty. Each group's colors are ascending; allocation fields remain zero.
/// Builds one boundary mask and one dense ownership lookup, then scans the
/// occupied tiles once. Validated Pool memberships are disjoint, so summing
/// different groups cannot double-count a pixel.
pub fn pool_all_group_counts(
    entity: &PlaceableEntity,
) -> Result<BTreeMap<u32, Vec<DistributionColorSummary>>, DistributionError> {
    let blind = entity
        .as_blind()
        .ok_or_else(|| DistributionError::NotPool(entity.id().clone()))?;
    let playable = crate::pool_boundary::pool_playable_mask(entity).map_err(|error| {
        DistributionError::InvalidPoolBoundary {
            entity_id: entity.id().clone(),
            reason: error.to_string(),
        }
    })?;
    let resolution = blind.pixels_per_cell();
    let width = usize::from(entity.shape().width()) * usize::from(resolution);
    let height = usize::from(entity.shape().height()) * usize::from(resolution);
    let mut owners = vec![0_u32; width * height];
    let mut groups = BTreeMap::<u32, BTreeMap<u16, DistributionColorSummary>>::new();
    groups.insert(0, BTreeMap::new());
    for group in blind.distribution_groups() {
        groups.insert(group.id, BTreeMap::new());
        for pixel in &group.pixels {
            owners[usize::from(pixel.x) + usize::from(pixel.y) * width] = group.id;
        }
    }
    for (cell, tile) in entity.shape().occupied_cells().zip(blind.tiles()) {
        for y in 0..resolution {
            for x in 0..resolution {
                let color = tile.color(x, y).unwrap_or_default();
                if color == 0 {
                    continue;
                }
                let pixel = BlindPixel::new(
                    u16::from(cell.x) * u16::from(resolution) + u16::from(x),
                    u16::from(cell.y) * u16::from(resolution) + u16::from(y),
                );
                let owner = owners[usize::from(pixel.x) + usize::from(pixel.y) * width];
                let color = u16::from(color);
                let summary = groups
                    .entry(owner)
                    .or_default()
                    .entry(color)
                    .or_insert_with(|| DistributionColorSummary::empty(color));
                summary.painted += 1;
                if playable.contains(pixel) {
                    summary.effective += 1;
                }
            }
        }
    }
    Ok(groups
        .into_iter()
        .map(|(id, colors)| (id, colors.into_values().collect()))
        .collect())
}

/// Computes an atomic plan without changing the snapshot or any target color.
/// Unlocked Unlimited rows become finite. Locked rows reserve their unchanged
/// finite capacity. Positive remaining budgets require positive unlocked weight.
/// Integer largest-remainder ties break by entity ID, then layer index.
pub fn distribution_plan(
    snapshot: &LevelSnapshot,
    request: &DistributionRequest,
) -> Result<DistributionPlan, DistributionError> {
    if request.pools.is_empty() {
        return Err(DistributionError::EmptyPools);
    }
    if request.layers.is_empty() {
        return Err(DistributionError::EmptyLayers);
    }
    if request.pools.len() > MAX_DISTRIBUTION_POOLS {
        return Err(DistributionError::TooManyPools {
            actual: request.pools.len(),
        });
    }
    if request.layers.len() > MAX_DISTRIBUTION_LAYERS {
        return Err(DistributionError::TooManyLayers {
            actual: request.layers.len(),
        });
    }
    let mut colors = BTreeMap::<u16, DistributionColorSummary>::new();
    let mut pool_ids = BTreeSet::new();
    for pool in &request.pools {
        if !pool_ids.insert(&pool.entity_id) {
            return Err(DistributionError::DuplicatePool(pool.entity_id.clone()));
        }
        let entity = snapshot
            .entity(&pool.entity_id)
            .ok_or_else(|| DistributionError::EntityNotFound(pool.entity_id.clone()))?;
        for (color, count) in count_pool_groups(entity, &pool.group_ids)? {
            let summary = colors
                .entry(color)
                .or_insert_with(|| DistributionColorSummary::empty(color));
            summary.painted += count.painted;
            summary.effective += count.effective;
        }
    }
    let mut selections = request.layers.iter().collect::<Vec<_>>();
    selections.sort_by_key(|selection| (&selection.entity_id, selection.layer_index));
    let mut pairs = BTreeSet::new();
    let mut layers = Vec::with_capacity(selections.len());
    let mut by_color = BTreeMap::<u16, Vec<usize>>::new();
    for selection in selections {
        let entity_id = &selection.entity_id;
        let layer_index = selection.layer_index;
        if !pairs.insert((entity_id, layer_index)) {
            return Err(DistributionError::DuplicateLayer {
                entity_id: entity_id.clone(),
                layer_index,
            });
        }
        if selection.weight > MAX_DISTRIBUTION_WEIGHT {
            return Err(DistributionError::WeightTooLarge {
                entity_id: entity_id.clone(),
                layer_index,
                weight: selection.weight,
            });
        }
        let entity = snapshot
            .entity(entity_id)
            .ok_or_else(|| DistributionError::EntityNotFound(entity_id.clone()))?;
        let PlaceableEntityKind::Block(block) = entity.kind() else {
            return Err(DistributionError::NotBlock(entity_id.clone()));
        };
        let layer = block.collect_layers().get(layer_index).ok_or_else(|| {
            DistributionError::LayerNotFound {
                entity_id: entity_id.clone(),
                layer_index,
            }
        })?;
        let color = layer.color_index();
        if color > MAX_COLLECT_COLOR_INDEX {
            return Err(DistributionError::InvalidTargetColor {
                entity_id: entity_id.clone(),
                layer_index,
                color_index: color,
            });
        }
        let capacity = if layer.is_locked() {
            match layer.capacity() {
                CollectCapacity::Finite(value) => value,
                CollectCapacity::Unlimited => {
                    return Err(DistributionError::LockedUnlimited {
                        entity_id: entity_id.clone(),
                        layer_index,
                    });
                }
            }
        } else {
            0
        };
        let summary = colors
            .entry(color)
            .or_insert_with(|| DistributionColorSummary::empty(color));
        summary.locked += u64::from(capacity);
        by_color.entry(color).or_default().push(layers.len());
        layers.push(DistributionLayerAllocation {
            entity_id: entity_id.clone(),
            layer_index,
            before: layer.capacity(),
            capacity,
            weight: selection.weight,
            locked: layer.is_locked(),
        });
    }
    for (&color, summary) in &mut colors {
        let Some(indices) = by_color.get(&color) else {
            summary.unmatched = summary.effective;
            continue;
        };
        if summary.locked > summary.effective {
            return Err(DistributionError::LockedExceedsBudget {
                color_index: color,
                locked: summary.locked,
                effective: summary.effective,
            });
        }
        allocate_color(
            color,
            summary.effective - summary.locked,
            indices,
            &mut layers,
        )?;
        summary.allocated = summary.effective;
    }
    Ok(DistributionPlan {
        colors: colors.into_values().collect(),
        layers,
    })
}

fn allocate_color(
    color_index: u16,
    remaining: u64,
    indices: &[usize],
    layers: &mut [DistributionLayerAllocation],
) -> Result<(), DistributionError> {
    let total_weight: u64 = indices
        .iter()
        .filter(|&&index| !layers[index].locked)
        .map(|&index| u64::from(layers[index].weight))
        .sum();
    if remaining > 0 && total_weight == 0 {
        return Err(DistributionError::ZeroTotalWeight {
            color_index,
            remaining,
        });
    }
    let mut remainders = Vec::new();
    let mut assigned = 0_u64;
    for &index in indices {
        if layers[index].locked {
            continue;
        }
        let product = u128::from(remaining) * u128::from(layers[index].weight);
        let (capacity, remainder) = if total_weight == 0 {
            (0, 0)
        } else {
            (
                (product / u128::from(total_weight)) as u64,
                product % u128::from(total_weight),
            )
        };
        set_capacity(&mut layers[index], capacity)?;
        assigned += capacity;
        if layers[index].weight > 0 {
            remainders.push((index, remainder));
        }
    }
    remainders.sort_by(|&(left, left_remainder), &(right, right_remainder)| {
        right_remainder
            .cmp(&left_remainder)
            .then_with(|| layers[left].entity_id.cmp(&layers[right].entity_id))
            .then_with(|| layers[left].layer_index.cmp(&layers[right].layer_index))
    });
    for &(index, _) in remainders.iter().take((remaining - assigned) as usize) {
        let capacity = u64::from(layers[index].capacity) + 1;
        set_capacity(&mut layers[index], capacity)?;
    }
    Ok(())
}

fn set_capacity(
    layer: &mut DistributionLayerAllocation,
    capacity: u64,
) -> Result<(), DistributionError> {
    if capacity > u64::from(MAX_COLLECT_CAPACITY) {
        return Err(DistributionError::CapacityOverflow {
            entity_id: layer.entity_id.clone(),
            layer_index: layer.layer_index,
            capacity,
        });
    }
    layer.capacity = capacity as u32;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_budget_overflow_is_rejected_including_remainder_increment() {
        // Current source limits cannot produce this much sand, but the arithmetic
        // must remain safe if those bounds grow in the future.
        let row = DistributionLayerAllocation {
            entity_id: "a".into(),
            layer_index: 0,
            before: CollectCapacity::Unlimited,
            capacity: 0,
            weight: 1,
            locked: false,
        };
        let mut rows = vec![row.clone()];
        assert!(matches!(
            allocate_color(1, u64::from(MAX_COLLECT_CAPACITY) + 1, &[0], &mut rows),
            Err(DistributionError::CapacityOverflow { .. })
        ));
        let mut rows = vec![
            row.clone(),
            DistributionLayerAllocation {
                entity_id: "b".into(),
                ..row
            },
        ];
        assert!(matches!(
            allocate_color(
                1,
                u64::from(MAX_COLLECT_CAPACITY) * 2 + 1,
                &[0, 1],
                &mut rows
            ),
            Err(DistributionError::CapacityOverflow { .. })
        ));
    }
}
