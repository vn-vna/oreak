use oreak_core::distribution::{
    DistributionError, DistributionLayerSelection, DistributionPlan, DistributionPoolSelection,
    DistributionRequest, MAX_DISTRIBUTION_LAYERS, MAX_DISTRIBUTION_POOLS, MAX_DISTRIBUTION_WEIGHT,
    distribution_plan, pool_all_group_counts, pool_group_counts,
};
use oreak_core::{
    Blind, BlindPixel, BlindTile, Block, CellKind, CollectCapacity, CollectLayer, GridPoint,
    GridSize, LevelSnapshot, PlaceableEntity, PoolBoundary, PoolDistributionGroup, Shape,
};

fn pool(id: &str, colors: &[u8]) -> PlaceableEntity {
    assert!(colors.len() <= 16);
    let mut pixels = vec![0; 16];
    pixels[..colors.len()].copy_from_slice(colors);
    let blind = Blind::new(4, vec![BlindTile::from_colors(4, pixels).unwrap()])
        .unwrap()
        .with_boundary(PoolBoundary {
            padding_pixels: Some(0),
            corner_radius_pixels: Some(0),
        })
        .unwrap();
    PlaceableEntity::blind(
        id,
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        blind,
    )
    .unwrap()
}

fn block(id: &str, rows: &[(u16, CollectCapacity, bool)]) -> PlaceableEntity {
    PlaceableEntity::block(
        id,
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        Block::new(
            rows.iter()
                .map(|&(color, capacity, locked)| {
                    CollectLayer::new(color, Some(3), capacity, locked)
                })
                .collect(),
        ),
    )
    .unwrap()
}

fn target(id: &str, color: u16) -> PlaceableEntity {
    block(id, &[(color, CollectCapacity::Finite(99), false)])
}

fn snapshot(entities: Vec<PlaceableEntity>) -> LevelSnapshot {
    let size = GridSize::new(32, 32).unwrap();
    let entities = entities
        .into_iter()
        .enumerate()
        .map(|(i, entity)| entity.moved_to(GridPoint::new((i % 32) as u16, (i / 32) as u16)))
        .collect();
    LevelSnapshot::from_parts(
        size,
        vec![CellKind::Floor; size.cell_count()],
        entities,
        vec![],
    )
    .unwrap()
}

fn request(pools: &[(&str, &[u32])], layers: &[(&str, usize, u32)]) -> DistributionRequest {
    DistributionRequest {
        pools: pools
            .iter()
            .map(|(id, groups)| DistributionPoolSelection {
                entity_id: (*id).into(),
                group_ids: groups.to_vec(),
            })
            .collect(),
        layers: layers
            .iter()
            .map(|&(id, layer_index, weight)| DistributionLayerSelection {
                entity_id: id.into(),
                layer_index,
                weight,
            })
            .collect(),
    }
}

fn capacities(plan: &DistributionPlan) -> Vec<u32> {
    plan.layers.iter().map(|row| row.capacity).collect()
}

#[test]
fn equal_weights_use_stable_id_then_layer_index_remainders() {
    let snapshot = snapshot(vec![
        pool("p", &[1; 5]),
        block(
            "a",
            &[
                (1, CollectCapacity::Finite(50), false),
                (1, CollectCapacity::Finite(20), false),
            ],
        ),
        target("b", 1),
    ]);
    let request = request(&[("p", &[0])], &[("b", 0, 1), ("a", 1, 1), ("a", 0, 1)]);
    let plan = distribution_plan(&snapshot, &request).unwrap();
    assert_eq!(capacities(&plan), vec![2, 2, 1]);
    assert_eq!(plan.layers[0].entity_id.as_str(), "a");
    assert_eq!(plan.layers[0].layer_index, 0);
    assert_eq!(plan.layers[1].layer_index, 1);
    assert_eq!(plan.colors[0].allocated, 5);
    let mut reordered = request.clone();
    reordered.layers.reverse();
    assert_eq!(distribution_plan(&snapshot, &reordered).unwrap(), plan);
}

#[test]
fn custom_weights_and_individual_zero_receive_exact_integer_totals() {
    let snapshot = snapshot(vec![
        pool("p", &[1; 10]),
        target("a", 1),
        target("b", 1),
        target("c", 1),
    ]);
    let plan = distribution_plan(
        &snapshot,
        &request(&[("p", &[0])], &[("a", 0, 1), ("b", 0, 3), ("c", 0, 0)]),
    )
    .unwrap();
    assert_eq!(capacities(&plan), vec![3, 7, 0]);
    assert_eq!(
        plan.layers
            .iter()
            .map(|row| u64::from(row.capacity))
            .sum::<u64>(),
        10
    );
    let plan = distribution_plan(
        &snapshot,
        &request(
            &[("p", &[0])],
            &[
                ("a", 0, MAX_DISTRIBUTION_WEIGHT),
                ("b", 0, MAX_DISTRIBUTION_WEIGHT),
            ],
        ),
    )
    .unwrap();
    assert_eq!(capacities(&plan), vec![5, 5]);
}

#[test]
fn multiple_pool_groups_are_unioned_and_default_excludes_unselected_groups() {
    let p = pool("p", &[1, 1, 1, 1, 2, 2, 0, 2]);
    let p = p
        .with_pool_distribution_groups(vec![
            PoolDistributionGroup {
                id: 8,
                name: "First".into(),
                pixels: vec![BlindPixel::new(0, 0), BlindPixel::new(1, 0)],
            },
            PoolDistributionGroup {
                id: 9,
                name: "Second".into(),
                pixels: vec![BlindPixel::new(2, 0), BlindPixel::new(0, 1)],
            },
        ])
        .unwrap();
    let default = pool_group_counts(&p, 0).unwrap();
    assert_eq!(
        default
            .iter()
            .map(|row| (row.color_index, row.painted))
            .collect::<Vec<_>>(),
        vec![(1, 1), (2, 2)]
    );
    assert!(
        default
            .iter()
            .all(|row| row.allocated == 0 && row.locked == 0 && row.unmatched == 0)
    );
    assert_eq!(pool_group_counts(&p, 8).unwrap()[0].effective, 2);
    let snapshot = snapshot(vec![
        p,
        pool("q", &[1, 2, 2]),
        target("a", 1),
        target("b", 2),
    ]);
    let selected = request(&[("p", &[8, 0]), ("q", &[0])], &[("a", 0, 1), ("b", 0, 1)]);
    assert_eq!(
        capacities(&distribution_plan(&snapshot, &selected).unwrap()),
        vec![4, 4]
    );
    let all = request(
        &[("p", &[9, 8, 0]), ("q", &[0])],
        &[("a", 0, 1), ("b", 0, 1)],
    );
    let plan = distribution_plan(&snapshot, &all).unwrap();
    assert_eq!(capacities(&plan), vec![5, 5]);
    let mut reordered = all;
    reordered.pools.reverse();
    reordered.pools[1].group_ids.reverse();
    assert_eq!(distribution_plan(&snapshot, &reordered).unwrap(), plan);
}

#[test]
fn locked_finite_rows_reserve_budget_without_using_their_weight() {
    let snapshot = snapshot(vec![
        pool("p", &[1; 10]),
        block("a", &[(1, CollectCapacity::Finite(3), true)]),
        target("b", 1),
        target("c", 1),
    ]);
    let plan = distribution_plan(
        &snapshot,
        &request(
            &[("p", &[0])],
            &[("a", 0, 1_000_000), ("b", 0, 1), ("c", 0, 1)],
        ),
    )
    .unwrap();
    assert_eq!(capacities(&plan), vec![3, 4, 3]);
    assert!(plan.layers[0].locked);
    assert_eq!(plan.layers[0].before, CollectCapacity::Finite(3));
    assert_eq!((plan.colors[0].locked, plan.colors[0].allocated), (3, 10));
}

#[test]
fn locked_rows_over_budget_and_locked_unlimited_are_errors() {
    for capacity in [CollectCapacity::Finite(6), CollectCapacity::Unlimited] {
        let snapshot = snapshot(vec![pool("p", &[1; 5]), block("a", &[(1, capacity, true)])]);
        let error =
            distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 0)])).unwrap_err();
        match capacity {
            CollectCapacity::Finite(_) => assert!(matches!(
                error,
                DistributionError::LockedExceedsBudget {
                    color_index: 1,
                    locked: 6,
                    effective: 5
                }
            )),
            CollectCapacity::Unlimited => {
                assert!(matches!(error, DistributionError::LockedUnlimited { .. }))
            }
        }
    }
}

#[test]
fn locked_budget_exactly_consumed_allows_zero_unlocked_weight() {
    let snapshot = snapshot(vec![
        pool("p", &[1; 5]),
        block("a", &[(1, CollectCapacity::Finite(5), true)]),
        target("b", 1),
    ]);
    let plan = distribution_plan(
        &snapshot,
        &request(&[("p", &[0])], &[("a", 0, 0), ("b", 0, 0)]),
    )
    .unwrap();
    assert_eq!(capacities(&plan), vec![5, 0]);
    assert_eq!(
        capacities(
            &distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 0)])).unwrap()
        ),
        vec![5]
    );
}

#[test]
fn unlocked_unlimited_becomes_finite_and_snapshot_is_never_changed() {
    let snapshot = snapshot(vec![
        pool("p", &[1; 5]),
        block("a", &[(1, CollectCapacity::Unlimited, false)]),
    ]);
    let before = snapshot.clone();
    let hash = snapshot.content_hash();
    let plan = distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 1)])).unwrap();
    assert_eq!(plan.layers[0].before, CollectCapacity::Unlimited);
    assert_eq!(capacities(&plan), vec![5]);
    assert_eq!(snapshot, before);
    assert_eq!(snapshot.content_hash(), hash);
    assert!(distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 0)])).is_err());
    assert_eq!(snapshot, before);
}

#[test]
fn positive_budget_requires_positive_unlocked_weight() {
    let snapshot = snapshot(vec![
        pool("p", &[1; 5]),
        target("a", 1),
        block("b", &[(1, CollectCapacity::Finite(3), true)]),
    ]);
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 0)])),
        Err(DistributionError::ZeroTotalWeight {
            color_index: 1,
            remaining: 5
        })
    ));
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("b", 0, 10)])),
        Err(DistributionError::ZeroTotalWeight {
            color_index: 1,
            remaining: 2
        })
    ));
}

#[test]
fn zero_budget_accepts_zero_weights_and_zeros_unlocked_rows() {
    let snapshot = snapshot(vec![
        pool("p", &[0; 16]),
        target("a", 1),
        block(
            "b",
            &[
                (1, CollectCapacity::Unlimited, false),
                (2, CollectCapacity::Finite(0), true),
            ],
        ),
    ]);
    let plan = distribution_plan(
        &snapshot,
        &request(&[("p", &[0])], &[("a", 0, 0), ("b", 0, 0), ("b", 1, 0)]),
    )
    .unwrap();
    assert_eq!(capacities(&plan), vec![0, 0, 0]);
    assert!(
        plan.colors
            .iter()
            .all(|row| row.painted == 0 && row.effective == 0)
    );
}

#[test]
fn mixed_colors_report_unmatched_without_recoloring_and_zero_target_only_colors() {
    let snapshot = snapshot(vec![
        pool("p", &[1, 1, 2, 2, 2, 0]),
        target("a", 1),
        target("b", 0),
        target("c", 15),
    ]);
    let plan = distribution_plan(
        &snapshot,
        &request(&[("p", &[0])], &[("a", 0, 1), ("b", 0, 1), ("c", 0, 0)]),
    )
    .unwrap();
    assert_eq!(capacities(&plan), vec![2, 0, 0]);
    assert_eq!(
        plan.colors
            .iter()
            .map(|row| row.color_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 15]
    );
    assert_eq!(plan.colors[2].unmatched, 3);
    for row in &plan.colors {
        assert_eq!(row.effective, row.allocated + row.unmatched);
    }
    let oreak_core::PlaceableEntityKind::Block(block) =
        snapshot.entity(&"b".into()).unwrap().kind()
    else {
        panic!()
    };
    assert_eq!(block.collect_layers()[0].color_index(), 0);
}

#[test]
fn disabled_target_color_is_an_explicit_error() {
    let snapshot = snapshot(vec![pool("p", &[1]), target("a", u16::MAX)]);
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 1)])),
        Err(DistributionError::InvalidTargetColor {
            color_index: u16::MAX,
            ..
        })
    ));
}

#[test]
fn empty_requests_and_empty_group_selections_are_rejected() {
    let snapshot = snapshot(vec![pool("p", &[1]), target("a", 1)]);
    assert_eq!(
        distribution_plan(&snapshot, &request(&[], &[("a", 0, 1)])),
        Err(DistributionError::EmptyPools)
    );
    assert_eq!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[])),
        Err(DistributionError::EmptyLayers)
    );
    assert_eq!(
        distribution_plan(&snapshot, &request(&[("p", &[])], &[("a", 0, 1)])),
        Err(DistributionError::EmptyGroups("p".into()))
    );
}

#[test]
fn request_size_and_weight_bounds_are_checked() {
    let snapshot = snapshot(vec![pool("p", &[1]), target("a", 1)]);
    let mut selected = request(&[("p", &[0])], &[("a", 0, 1)]);
    selected.pools = vec![selected.pools[0].clone(); MAX_DISTRIBUTION_POOLS + 1];
    assert!(matches!(
        distribution_plan(&snapshot, &selected),
        Err(DistributionError::TooManyPools { .. })
    ));
    let mut selected = request(&[("p", &[0])], &[("a", 0, 1)]);
    selected.layers = vec![selected.layers[0].clone(); MAX_DISTRIBUTION_LAYERS + 1];
    assert!(matches!(
        distribution_plan(&snapshot, &selected),
        Err(DistributionError::TooManyLayers { .. })
    ));
    assert!(matches!(
        distribution_plan(
            &snapshot,
            &request(&[("p", &[0])], &[("a", 0, MAX_DISTRIBUTION_WEIGHT + 1)])
        ),
        Err(DistributionError::WeightTooLarge { .. })
    ));
}

#[test]
fn duplicate_pool_group_and_target_pairs_are_rejected() {
    let snapshot = snapshot(vec![pool("p", &[1]), target("a", 1)]);
    assert!(matches!(
        distribution_plan(
            &snapshot,
            &request(&[("p", &[0]), ("p", &[0])], &[("a", 0, 1)])
        ),
        Err(DistributionError::DuplicatePool(_))
    ));
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0, 0])], &[("a", 0, 1)])),
        Err(DistributionError::DuplicateGroup { .. })
    ));
    assert!(matches!(
        distribution_plan(
            &snapshot,
            &request(&[("p", &[0])], &[("a", 0, 1), ("a", 0, 2)])
        ),
        Err(DistributionError::DuplicateLayer { .. })
    ));
}

#[test]
fn missing_entities_wrong_types_indices_and_group_ids_are_errors() {
    let snapshot = snapshot(vec![pool("p", &[1]), target("a", 1)]);
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("missing", &[0])], &[("a", 0, 1)])),
        Err(DistributionError::EntityNotFound(_))
    ));
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("missing", 0, 1)])),
        Err(DistributionError::EntityNotFound(_))
    ));
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("a", &[0])], &[("a", 0, 1)])),
        Err(DistributionError::NotPool(_))
    ));
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("p", 0, 1)])),
        Err(DistributionError::NotBlock(_))
    ));
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", usize::MAX, 1)])),
        Err(DistributionError::LayerNotFound { .. })
    ));
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[99])], &[("a", 0, 1)])),
        Err(DistributionError::GroupNotFound { group_id: 99, .. })
    ));
    assert!(matches!(
        pool_group_counts(snapshot.entity(&"p".into()).unwrap(), 99),
        Err(DistributionError::GroupNotFound { .. })
    ));
    assert!(matches!(
        pool_group_counts(snapshot.entity(&"a".into()).unwrap(), 0),
        Err(DistributionError::NotPool(_))
    ));
}

#[test]
fn boundary_pixels_are_painted_but_not_effective_in_both_public_apis() {
    let p = PlaceableEntity::blind(
        "p",
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(32, vec![BlindTile::from_colors(32, vec![1; 1024]).unwrap()]).unwrap(),
    )
    .unwrap();
    let counts = pool_group_counts(&p, 0).unwrap();
    assert_eq!((counts[0].painted, counts[0].effective), (1024, 780));
    let snapshot = snapshot(vec![p, target("a", 1)]);
    let plan = distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 1)])).unwrap();
    assert_eq!(
        (plan.colors[0].painted, plan.colors[0].effective),
        (1024, 780)
    );
    assert_eq!(capacities(&plan), vec![780]);
}

#[test]
fn invalid_boundary_geometry_is_an_error_instead_of_an_empty_budget() {
    let p = PlaceableEntity::blind(
        "p",
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(1, vec![BlindTile::from_colors(1, vec![1]).unwrap()]).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        pool_group_counts(&p, 0),
        Err(DistributionError::InvalidPoolBoundary { .. })
    ));
    let snapshot = snapshot(vec![p, target("a", 1)]);
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 1)])),
        Err(DistributionError::InvalidPoolBoundary { .. })
    ));
}

#[test]
fn source_budget_cannot_pay_a_different_colors_locked_reservation() {
    let snapshot = snapshot(vec![
        pool("p", &[1; 16]),
        block("a", &[(2, CollectCapacity::Finite(1), true)]),
    ]);
    assert!(matches!(
        distribution_plan(&snapshot, &request(&[("p", &[0])], &[("a", 0, 1)])),
        Err(DistributionError::LockedExceedsBudget {
            color_index: 2,
            locked: 1,
            effective: 0
        })
    ));
}

#[test]
fn small_budget_weight_combinations_conserve_all_sand() {
    for budget in 0..=16 {
        let snapshot = snapshot(vec![
            pool("p", &vec![1; budget]),
            target("a", 1),
            target("b", 1),
            target("c", 1),
        ]);
        for a in 0..=3 {
            for b in 0..=3 {
                for c in 0..=3 {
                    let result = distribution_plan(
                        &snapshot,
                        &request(&[("p", &[0])], &[("a", 0, a), ("b", 0, b), ("c", 0, c)]),
                    );
                    if budget > 0 && a + b + c == 0 {
                        assert!(matches!(
                            result,
                            Err(DistributionError::ZeroTotalWeight { .. })
                        ));
                        continue;
                    }
                    let plan = result.unwrap();
                    assert_eq!(
                        plan.layers.iter().map(|row| row.capacity).sum::<u32>(),
                        budget as u32
                    );
                    assert!(
                        plan.layers
                            .iter()
                            .all(|row| row.weight != 0 || row.capacity == 0)
                    );
                    for row in plan.layers {
                        if let Some(floor) = (budget as u32 * row.weight).checked_div(a + b + c) {
                            assert!(row.capacity == floor || row.capacity == floor + 1);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn bulk_group_counts_match_individual_counts_and_include_empty_groups() {
    let p = pool("p", &[1, 1, 2, 0, 2, 2, 1, 0, 1, 3, 3]);
    let p = PlaceableEntity::blind(
        "p",
        p.origin(),
        p.shape(),
        p.as_blind()
            .unwrap()
            .with_boundary(PoolBoundary {
                padding_pixels: Some(1),
                corner_radius_pixels: Some(0),
            })
            .unwrap(),
    )
    .unwrap()
    .with_pool_distribution_groups(vec![
        PoolDistributionGroup {
            id: 7,
            name: "Mixed".into(),
            pixels: vec![BlindPixel::new(0, 0), BlindPixel::new(1, 1)],
        },
        PoolDistributionGroup {
            id: 8,
            name: "Interior".into(),
            pixels: vec![BlindPixel::new(2, 1), BlindPixel::new(1, 2)],
        },
        PoolDistributionGroup {
            id: 9,
            name: "Empty".into(),
            pixels: vec![],
        },
        PoolDistributionGroup {
            id: 10,
            name: "Unpainted".into(),
            pixels: vec![BlindPixel::new(3, 0)],
        },
    ])
    .unwrap();
    let all = pool_all_group_counts(&p).unwrap();
    assert_eq!(
        all.keys().copied().collect::<Vec<_>>(),
        vec![0, 7, 8, 9, 10]
    );
    assert!(all[&9].is_empty());
    assert!(all[&10].is_empty());
    for (&id, counts) in &all {
        assert_eq!(*counts, pool_group_counts(&p, id).unwrap());
        assert!(
            counts
                .iter()
                .all(|row| row.locked == 0 && row.allocated == 0 && row.unmatched == 0)
        );
    }
    let tuples = |id| {
        all[&id]
            .iter()
            .map(|row| (row.color_index, row.painted, row.effective))
            .collect::<Vec<_>>()
    };
    assert_eq!(tuples(0), vec![(1, 2, 0), (2, 2, 0), (3, 1, 1)]);
    assert_eq!(tuples(7), vec![(1, 1, 0), (2, 1, 1)]);
    assert_eq!(tuples(8), vec![(1, 1, 1), (3, 1, 1)]);
    assert_eq!(
        all.values().flatten().map(|row| row.painted).sum::<u64>(),
        9
    );
    assert_eq!(
        all.values().flatten().map(|row| row.effective).sum::<u64>(),
        4
    );
}

#[test]
fn bulk_counter_keeps_default_for_empty_pools_and_reports_invalid_inputs() {
    let all = pool_all_group_counts(&pool("p", &[])).unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[&0].is_empty());
    assert!(matches!(
        pool_all_group_counts(&target("a", 1)),
        Err(DistributionError::NotPool(_))
    ));
    let p = PlaceableEntity::blind(
        "p",
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(1, vec![BlindTile::from_colors(1, vec![1]).unwrap()]).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        pool_all_group_counts(&p),
        Err(DistributionError::InvalidPoolBoundary { .. })
    ));
}

#[test]
fn public_request_and_plan_round_trip_through_serde() {
    let snapshot = snapshot(vec![pool("p", &[1; 5]), target("a", 1)]);
    let request = request(&[("p", &[0])], &[("a", 0, 1)]);
    assert_eq!(
        serde_json::from_str::<DistributionRequest>(&serde_json::to_string(&request).unwrap())
            .unwrap(),
        request
    );
    let plan = distribution_plan(&snapshot, &request).unwrap();
    assert_eq!(
        serde_json::from_str::<DistributionPlan>(&serde_json::to_string(&plan).unwrap()).unwrap(),
        plan
    );
}

#[test]
fn exact_selection_count_limits_are_accepted() {
    let mut entities = (0..MAX_DISTRIBUTION_POOLS)
        .map(|i| pool(&format!("p{i:02}"), &[1]))
        .collect::<Vec<_>>();
    entities.push(block(
        "a",
        &vec![(1, CollectCapacity::Finite(0), false); MAX_DISTRIBUTION_LAYERS],
    ));
    let snapshot = snapshot(entities);
    let request = DistributionRequest {
        pools: (0..MAX_DISTRIBUTION_POOLS)
            .map(|i| DistributionPoolSelection {
                entity_id: format!("p{i:02}").into(),
                group_ids: vec![0],
            })
            .collect(),
        layers: (0..MAX_DISTRIBUTION_LAYERS)
            .map(|i| DistributionLayerSelection {
                entity_id: "a".into(),
                layer_index: i,
                weight: 1,
            })
            .collect(),
    };
    let plan = distribution_plan(&snapshot, &request).unwrap();
    assert_eq!(plan.layers.len(), MAX_DISTRIBUTION_LAYERS);
    assert_eq!(plan.layers.iter().map(|row| row.capacity).sum::<u32>(), 64);
    assert!(plan.layers[..64].iter().all(|row| row.capacity == 1));
    assert!(plan.layers[64..].iter().all(|row| row.capacity == 0));
}
