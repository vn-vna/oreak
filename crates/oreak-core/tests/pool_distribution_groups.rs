use oreak_core::{
    Blind, BlindBrushOperation, BlindGuide, BlindGuideSet, BlindPixel, BlindStroke, BlindTile,
    CellKind, CommandEnvelope, CommandMetadata, EntityError, GridAnchor, GridPoint, GridSize,
    LevelCommand, LevelSnapshot, LevelTimeline, PlaceableEntity, PoolBoundary,
    PoolDistributionGroup, Shape,
};

fn group(id: u32, name: &str, pixels: &[(u16, u16)]) -> PoolDistributionGroup {
    PoolDistributionGroup {
        id,
        name: name.to_owned(),
        pixels: pixels.iter().map(|&(x, y)| BlindPixel::new(x, y)).collect(),
    }
}

fn empty_blind(shape: Shape, resolution: u8) -> Blind {
    Blind::new(
        resolution,
        vec![BlindTile::empty(resolution).unwrap(); shape.occupied_count() as usize],
    )
    .unwrap()
}

fn entity(shape: Shape, blind: Blind) -> PlaceableEntity {
    PlaceableEntity::blind("pool", GridPoint::new(0, 0), shape, blind).unwrap()
}

fn snapshot(entity: PlaceableEntity) -> LevelSnapshot {
    LevelSnapshot::from_parts(
        GridSize::new(8, 8).unwrap(),
        vec![CellKind::Floor; 64],
        vec![entity],
        vec![],
    )
    .unwrap()
}

#[test]
fn legacy_blind_json_and_hash_bytes_are_unchanged() {
    let json = r#"{"pixels_per_cell":2,"tiles":[{"pixels_per_cell":2,"bits":[0],"colors":[0,0,0,0]}],"guides":[]}"#;
    let blind: Blind = serde_json::from_str(json).unwrap();
    assert!(blind.distribution_groups().is_empty());
    assert_eq!(blind.boundary(), PoolBoundary::default());
    assert_eq!(serde_json::to_string(&blind).unwrap(), json);
    let shape = Shape::new(1, 1, 1).unwrap();
    let level = snapshot(entity(shape, blind));

    // Frozen pre-extension byte recipe for this concrete legacy snapshot.
    let mut old = blake3::Hasher::new();
    old.update(b"oreak-level-snapshot");
    old.update(&1_u32.to_le_bytes());
    old.update(&8_u16.to_le_bytes());
    old.update(&8_u16.to_le_bytes());
    old.update(&[0; 64]);
    old.update(b"oreak-level-domain-v1");
    old.update(&1_u64.to_le_bytes());
    old.update(&4_u64.to_le_bytes());
    old.update(b"pool");
    old.update(&[0; 4]); // origin
    old.update(&[1, 1]); // shape dimensions
    old.update(&1_u64.to_le_bytes()); // shape mask
    old.update(&[1, 2]); // Blind tag, resolution
    old.update(&1_u64.to_le_bytes()); // tile count
    old.update(&1_u64.to_le_bytes()); // occupancy byte count
    old.update(&[0]);
    old.update(&0_u64.to_le_bytes()); // decorator count
    assert_eq!(level.content_hash().as_bytes(), old.finalize().as_bytes());

    let old_json = serde_json::to_string(&level).unwrap();
    assert!(!old_json.contains("distribution_groups"));
    assert!(!old_json.contains("boundary"));
    let restored: LevelSnapshot = serde_json::from_str(&old_json).unwrap();
    assert_eq!(serde_json::to_string(&restored).unwrap(), old_json);
    assert_eq!(restored.content_hash(), level.content_hash());
}

#[test]
fn groups_are_canonical_persistent_and_independent_of_guides_and_paint() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let blind = empty_blind(shape, 3)
        .with_distribution_groups(
            shape,
            vec![
                group(9, "  Disconnected  ", &[(2, 2), (0, 0)]),
                group(2, "Empty region", &[]),
            ],
        )
        .unwrap();
    assert_eq!(
        blind.distribution_groups(),
        &[
            group(2, "Empty region", &[]),
            group(9, "Disconnected", &[(0, 0), (2, 2)]),
        ]
    );
    assert_eq!(blind.color_at(shape, BlindPixel::new(0, 0)), Some(0));
    assert_eq!(
        blind
            .paintable_partition(shape, BlindPixel::new(0, 0))
            .len(),
        9
    );
    let level = snapshot(entity(shape, blind));
    let json = serde_json::to_string(&level).unwrap();
    let restored: LevelSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, level);
    assert_eq!(restored.content_hash(), level.content_hash());
}

#[test]
fn rejects_reserved_duplicate_ids_invalid_names_and_too_many_groups() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let blind = empty_blind(shape, 2);
    assert!(matches!(
        blind.with_distribution_groups(shape, vec![group(0, "Default", &[])]),
        Err(EntityError::ReservedPoolDistributionGroupId)
    ));
    assert!(matches!(
        blind.with_distribution_groups(shape, vec![group(1, "A", &[]), group(1, "B", &[])]),
        Err(EntityError::DuplicatePoolDistributionGroupId(1))
    ));
    for name in ["  \n\t".to_owned(), "界".repeat(81)] {
        assert!(matches!(
            blind.with_distribution_groups(shape, vec![group(1, &name, &[])]),
            Err(EntityError::InvalidPoolDistributionGroupName(1))
        ));
    }
    assert!(
        blind
            .with_distribution_groups(shape, vec![group(1, &"界".repeat(80), &[])])
            .is_ok()
    );
    assert!(
        blind
            .with_distribution_groups(shape, (1..=64).map(|id| group(id, "A", &[])).collect())
            .is_ok()
    );
    assert!(matches!(
        blind.with_distribution_groups(shape, (1..=65).map(|id| group(id, "A", &[])).collect()),
        Err(EntityError::PoolDistributionGroupLimit { actual: 65 })
    ));
    assert!(blind.distribution_groups().is_empty());
}

#[test]
fn rejects_overlap_duplicate_pixels_bounds_and_shape_holes() {
    let shape = Shape::new(3, 3, 0b111_101_111).unwrap();
    let blind = empty_blind(shape, 2);
    for groups in [
        vec![group(1, "A", &[(0, 0), (0, 0)])],
        vec![group(1, "A", &[(0, 0)]), group(2, "B", &[(0, 0)])],
    ] {
        assert!(matches!(
            blind.with_distribution_groups(shape, groups),
            Err(EntityError::DuplicatePoolDistributionPixel(BlindPixel {
                x: 0,
                y: 0
            }))
        ));
    }
    for point in [(2, 2), (3, 3), (6, 0), (0, 6), (u16::MAX, 0)] {
        assert!(matches!(
            blind.with_distribution_groups(shape, vec![group(1, "A", &[point])]),
            Err(EntityError::PoolDistributionPixelOutsideFootprint(_))
        ));
    }
}

#[test]
fn snapshot_deserialization_validates_membership_and_canonicalizes_order() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let level = snapshot(entity(shape, empty_blind(shape, 2)));
    let base = serde_json::to_value(level).unwrap();
    for invalid in [
        serde_json::json!([{"id": 0, "name": "A", "pixels": []}]),
        serde_json::json!([{"id": 1, "name": "A", "pixels": [{"x": 2, "y": 0}]}]),
        serde_json::json!([{"id": 1, "name": "A", "pixels": [{"x": 0, "y": 0}, {"x": 0, "y": 0}]}]),
        serde_json::json!([{"id": 1, "name": "A", "pixels": [], "typo": true}]),
        serde_json::json!([{"id": 1, "name": " ", "pixels": []}]),
    ] {
        let mut value = base.clone();
        value["entities"][0]["kind"]["Blind"]["distribution_groups"] = invalid;
        assert!(serde_json::from_value::<LevelSnapshot>(value).is_err());
    }
    let mut value = base;
    value["entities"][0]["kind"]["Blind"]["distribution_groups"] =
        serde_json::to_value(vec![group(8, " B ", &[(1, 1), (0, 0)]), group(1, "A", &[])]).unwrap();
    let restored: LevelSnapshot = serde_json::from_value(value).unwrap();
    assert_eq!(
        restored.entities()[0]
            .as_blind()
            .unwrap()
            .distribution_groups(),
        &[group(1, "A", &[]), group(8, "B", &[(0, 0), (1, 1)]),]
    );
}

#[test]
fn painting_erasing_filling_and_guides_preserve_even_empty_pixel_membership() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let groups = vec![group(1, "Corners", &[(0, 0), (1, 1)])];
    let mut blind = empty_blind(shape, 2)
        .with_distribution_groups(shape, groups.clone())
        .unwrap();
    let stroke = BlindStroke::new(vec![BlindPixel::new(0, 0)]).unwrap();
    let guides = BlindGuideSet::new(vec![
        BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap(),
    ])
    .unwrap();
    for operation in [
        BlindBrushOperation::PaintStroke {
            color_index: 4,
            stroke: stroke.clone(),
        },
        BlindBrushOperation::EraseStroke { stroke },
        BlindBrushOperation::FloodFill {
            start: BlindPixel::new(0, 0),
            color_index: 6,
        },
        BlindBrushOperation::AddGuides {
            guides: guides.clone(),
        },
        BlindBrushOperation::RemoveGuides { guides },
    ] {
        blind = blind.apply_operation(shape, &operation).unwrap();
        assert_eq!(blind.distribution_groups(), groups);
    }
}

#[test]
fn rotation_flip_and_move_follow_pixel_coordinates_not_painted_occupancy() {
    let shape = Shape::new(2, 1, 0b11).unwrap();
    let groups = vec![
        group(1, "Corners", &[(0, 0), (3, 1)]),
        group(5, "Empty", &[]),
    ];
    let original = entity(
        shape,
        empty_blind(shape, 2)
            .with_distribution_groups(shape, groups.clone())
            .unwrap(),
    );
    let rotated = original.rotated_clockwise();
    assert_eq!(
        rotated.as_blind().unwrap().distribution_groups(),
        &[
            group(1, "Corners", &[(0, 3), (1, 0)]),
            group(5, "Empty", &[]),
        ]
    );
    let flipped = original.flipped_horizontal();
    assert_eq!(
        flipped.as_blind().unwrap().distribution_groups(),
        &[
            group(1, "Corners", &[(0, 1), (3, 0)]),
            group(5, "Empty", &[]),
        ]
    );
    assert_eq!(
        original,
        rotated
            .rotated_clockwise()
            .rotated_clockwise()
            .rotated_clockwise()
    );
    assert_eq!(original, flipped.flipped_horizontal());
    assert_eq!(
        original
            .moved_to(GridPoint::new(5, 5))
            .as_blind()
            .unwrap()
            .distribution_groups(),
        groups
    );
    assert!(
        serde_json::from_value::<PlaceableEntity>(serde_json::to_value(rotated).unwrap()).is_ok()
    );
}

#[test]
fn nearest_center_resampling_retains_names_and_includes_unpainted_pixels() {
    let shape = Shape::new(2, 1, 0b11).unwrap();
    let blind = empty_blind(shape, 2)
        .with_distribution_groups(
            shape,
            vec![
                group(1, "Lower", &[(0, 0), (2, 0)]),
                group(2, "Upper", &[(1, 1), (3, 1)]),
            ],
        )
        .unwrap();
    let enlarged = blind.resampled(shape, 4).unwrap();
    assert_eq!(
        enlarged.distribution_groups(),
        &[
            group(
                1,
                "Lower",
                &[
                    (0, 0),
                    (0, 1),
                    (1, 0),
                    (1, 1),
                    (4, 0),
                    (4, 1),
                    (5, 0),
                    (5, 1)
                ]
            ),
            group(
                2,
                "Upper",
                &[
                    (2, 2),
                    (2, 3),
                    (3, 2),
                    (3, 3),
                    (6, 2),
                    (6, 3),
                    (7, 2),
                    (7, 3)
                ]
            ),
        ]
    );
    assert_eq!(enlarged.resampled(shape, 2).unwrap(), blind);
    let reduced = blind.resampled(shape, 1).unwrap();
    assert_eq!(
        reduced.distribution_groups(),
        &[group(1, "Lower", &[]), group(2, "Upper", &[(0, 0), (1, 0)]),]
    );
    assert_eq!(reduced.tiles()[0].colors(), &[0]);
}

#[test]
fn grid_resize_preserves_local_groups_and_boundary() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let blind = empty_blind(shape, 2)
        .with_distribution_groups(shape, vec![group(1, "A", &[(0, 0)])])
        .unwrap();
    let original = entity(shape, blind);
    let mut timeline = LevelTimeline::new(snapshot(original.clone())).unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("resize", "test", 1),
            LevelCommand::ResizeGrid {
                size: GridSize::new(10, 10).unwrap(),
                anchor: GridAnchor::Center,
            },
        ))
        .unwrap();
    assert_eq!(
        timeline.snapshot().entities()[0].as_blind(),
        original.as_blind()
    );
}

#[test]
fn canonical_hash_is_order_independent_and_encodes_ids_names_and_membership() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let blind = empty_blind(shape, 2);
    let groups = vec![group(1, "A", &[(0, 0), (1, 1)]), group(2, "B", &[])];
    let hash = |groups| {
        snapshot(entity(
            shape,
            blind.with_distribution_groups(shape, groups).unwrap(),
        ))
        .content_hash()
    };
    let expected = hash(groups.clone());
    assert_eq!(
        hash(vec![group(2, "B", &[]), group(1, " A ", &[(1, 1), (0, 0)])]),
        expected
    );
    for other in [
        vec![],
        vec![group(1, "A", &[(0, 0), (1, 1)])],
        vec![group(3, "A", &[(0, 0), (1, 1)]), group(2, "B", &[])],
        vec![group(1, "AA", &[(0, 0), (1, 1)]), group(2, "B", &[])],
        vec![group(1, "A", &[(0, 0)]), group(2, "B", &[(1, 1)])],
        vec![group(1, "A", &[(0, 1), (1, 0)]), group(2, "B", &[])],
    ] {
        assert_ne!(hash(other), expected);
    }
    assert_ne!(
        hash(vec![group(1, "ab", &[]), group(2, "c", &[])]),
        hash(vec![group(1, "a", &[]), group(2, "bc", &[])])
    );
}

#[test]
fn boundary_defaults_validation_persistence_and_transform_preservation() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let blind = empty_blind(shape, 4);
    let boundary = PoolBoundary {
        padding_pixels: Some(0),
        corner_radius_pixels: Some(2),
    };
    let customized = blind.with_boundary(boundary).unwrap();
    assert_eq!(customized.boundary(), boundary);
    let original = entity(shape, customized);
    for transformed in [
        original.rotated_clockwise(),
        original.flipped_horizontal(),
        original.resampled_blind(8).unwrap(),
    ] {
        assert_eq!(transformed.as_blind().unwrap().boundary(), boundary);
    }
    let level = snapshot(original);
    let restored: LevelSnapshot =
        serde_json::from_str(&serde_json::to_string(&level).unwrap()).unwrap();
    assert_eq!(restored, level);
    let hash =
        |boundary| snapshot(entity(shape, blind.with_boundary(boundary).unwrap())).content_hash();
    assert_ne!(hash(boundary), hash(PoolBoundary::default()));
    assert_ne!(
        hash(boundary),
        hash(PoolBoundary {
            padding_pixels: None,
            corner_radius_pixels: Some(2)
        })
    );
    assert_ne!(
        hash(boundary),
        hash(PoolBoundary {
            padding_pixels: Some(2),
            corner_radius_pixels: Some(0)
        })
    );
    for bad in [
        PoolBoundary {
            padding_pixels: Some(1025),
            corner_radius_pixels: None,
        },
        PoolBoundary {
            padding_pixels: None,
            corner_radius_pixels: Some(1025),
        },
    ] {
        assert_eq!(
            blind.with_boundary(bad),
            Err(EntityError::InvalidPoolBoundary)
        );
        let mut value = serde_json::to_value(&level).unwrap();
        value["entities"][0]["kind"]["Blind"]["boundary"] = serde_json::to_value(bad).unwrap();
        assert!(serde_json::from_value::<LevelSnapshot>(value).is_err());
    }
}
