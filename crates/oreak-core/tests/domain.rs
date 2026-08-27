use oreak_core::{
    Blind, BlindTile, Block, CardinalDirection, CellKind, CollectCapacity, CollectLayer, Decorator,
    EntityError, GridPoint, GridSize, LevelError, LevelSnapshot, PlaceableEntity, Shape, ShapeCell,
    ShapeError,
};

fn l_shape() -> Shape {
    Shape::from_cells(&[
        ShapeCell::new(0, 0),
        ShapeCell::new(1, 0),
        ShapeCell::new(0, 1),
    ])
    .unwrap()
}

fn block(id: &str, origin: GridPoint, shape: Shape) -> PlaceableEntity {
    PlaceableEntity::block(id, origin, shape, Block::default()).unwrap()
}

fn floor_cells(size: GridSize) -> Vec<CellKind> {
    vec![CellKind::Floor; size.cell_count()]
}

#[test]
fn shapes_are_tight_connected_and_bounded() {
    assert_eq!(Shape::new(1, 1, 0), Err(ShapeError::Empty));
    assert!(matches!(
        Shape::new(9, 8, u64::MAX),
        Err(ShapeError::AreaTooLarge { .. })
    ));
    assert_eq!(Shape::new(2, 2, 0b0011), Err(ShapeError::NotTightlyBounded));
    assert_eq!(Shape::new(2, 2, 0b1001), Err(ShapeError::Disconnected));
    assert_eq!(
        Shape::from_cells(&[ShapeCell::new(0, 0), ShapeCell::new(0, 0)]),
        Err(ShapeError::DuplicateCell(ShapeCell::new(0, 0)))
    );

    let shape = l_shape();
    assert_eq!(shape.bounding_area(), 4);
    assert_eq!(shape.occupied_count(), 3);
    assert_eq!(shape.rotated_clockwise().occupied_mask(), 0b1011);
    assert_eq!(shape.flipped_horizontal().occupied_mask(), 0b1011);

    let full = Shape::new(8, 8, u64::MAX).unwrap();
    assert_eq!(full.occupied_cells().count(), 64);
}

#[test]
fn blocks_and_blinds_enforce_their_bounds() {
    let wide = Shape::new(9, 1, 0x1ff).unwrap();
    assert!(matches!(
        PlaceableEntity::block("wide", GridPoint::new(0, 0), wide, Block::default()),
        Err(EntityError::BlockShapeTooLarge { .. })
    ));

    assert!(matches!(
        BlindTile::empty(0),
        Err(EntityError::InvalidPixelsPerCell(0))
    ));
    assert_eq!(
        BlindTile::new(1, vec![0b10]),
        Err(EntityError::BlindTilePadding)
    );
    assert!(matches!(
        Blind::new(1, (0..65).map(|_| BlindTile::empty(1).unwrap()).collect()),
        Err(EntityError::BlindTileLimit { actual: 65 })
    ));

    let max_shape = Shape::new(64, 1, u64::MAX).unwrap();
    let tiles = (0..64).map(|_| BlindTile::empty(32).unwrap()).collect();
    let blind = Blind::new(32, tiles).unwrap();
    let entity = PlaceableEntity::blind("blind", GridPoint::new(0, 0), max_shape, blind).unwrap();
    let oreak_core::PlaceableEntityKind::Blind(blind) = entity.kind() else {
        panic!("expected a Blind entity");
    };
    assert_eq!(blind.tiles().len(), 64);
    assert_eq!(blind.tiles()[0].bits().len(), 128);

    let missing_tile = Blind::new(1, Vec::new()).unwrap();
    assert!(matches!(
        PlaceableEntity::blind(
            "bad-blind",
            GridPoint::new(0, 0),
            Shape::new(1, 1, 1).unwrap(),
            missing_tile
        ),
        Err(EntityError::BlindTileCount { .. })
    ));
}

#[test]
fn blind_tiles_follow_shape_transforms() {
    let tiles = vec![
        BlindTile::new(2, vec![0b0001]).unwrap(),
        BlindTile::new(2, vec![0b0010]).unwrap(),
        BlindTile::new(2, vec![0b0100]).unwrap(),
    ];
    let entity = PlaceableEntity::blind(
        "blind",
        GridPoint::new(5, 6),
        l_shape(),
        Blind::new(2, tiles).unwrap(),
    )
    .unwrap();

    let rotated = entity.rotated_clockwise();
    assert_eq!(rotated.id(), entity.id());
    assert_eq!(rotated.origin(), entity.origin());
    let oreak_core::PlaceableEntityKind::Blind(blind) = rotated.kind() else {
        panic!("expected a Blind entity");
    };
    assert_eq!(blind.tiles()[0].bits(), &[0b0001]);
    assert_eq!(blind.tiles()[1].bits(), &[0b0010]);
    assert_eq!(blind.tiles()[2].bits(), &[0b1000]);

    let rotated_four_times = entity
        .rotated_clockwise()
        .rotated_clockwise()
        .rotated_clockwise()
        .rotated_clockwise();
    assert_eq!(rotated_four_times, entity);
    assert_eq!(entity.flipped_horizontal().flipped_horizontal(), entity);
}

#[test]
fn snapshots_canonicalize_entities_and_decorators_for_hashing() {
    let size = GridSize::new(8, 8).unwrap();
    let a = block("a", GridPoint::new(0, 0), Shape::new(1, 1, 1).unwrap());
    let b = block("b", GridPoint::new(3, 3), l_shape());
    let ice = Decorator::ice("decorator-b", "b");
    let direction = Decorator::direction("decorator-a", "a", CardinalDirection::Right);

    let first = LevelSnapshot::from_parts(
        size,
        floor_cells(size),
        vec![b.clone(), a.clone()],
        vec![ice.clone(), direction.clone()],
    )
    .unwrap();
    let second =
        LevelSnapshot::from_parts(size, floor_cells(size), vec![a, b], vec![direction, ice])
            .unwrap();

    assert_eq!(first.entities()[0].id().as_str(), "a");
    assert_eq!(first.decorators()[0].id().as_str(), "decorator-a");
    assert_eq!(first.content_hash(), second.content_hash());

    let json = serde_json::to_string(&first).unwrap();
    let restored: LevelSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, first);
    assert_eq!(restored.content_hash(), first.content_hash());
}

#[test]
fn entity_free_snapshot_hash_retains_the_grid_hash_format() {
    let snapshot = LevelSnapshot::new(2, 2).unwrap();
    let mut expected = blake3::Hasher::new();
    expected.update(b"oreak-level-snapshot");
    expected.update(&1_u32.to_le_bytes());
    expected.update(&2_u16.to_le_bytes());
    expected.update(&2_u16.to_le_bytes());
    expected.update(&[0, 0, 0, 0]);

    assert_eq!(
        snapshot.content_hash().as_bytes(),
        expected.finalize().as_bytes()
    );
}

#[test]
fn snapshot_validation_rejects_collisions_and_bad_references() {
    let size = GridSize::new(4, 4).unwrap();
    let first = block("a", GridPoint::new(0, 0), l_shape());
    let overlapping = block("b", GridPoint::new(1, 0), Shape::new(1, 1, 1).unwrap());
    assert!(matches!(
        LevelSnapshot::from_parts(
            size,
            floor_cells(size),
            vec![first.clone(), overlapping],
            Vec::new()
        ),
        Err(LevelError::EntityOverlap { .. })
    ));

    let mut cells = floor_cells(size);
    cells[size.index_of(GridPoint::new(0, 0)).unwrap()] = CellKind::Wall;
    assert!(matches!(
        LevelSnapshot::from_parts(size, cells, vec![first.clone()], Vec::new()),
        Err(LevelError::EntityOnWall { .. })
    ));

    let out_of_bounds = block("edge", GridPoint::new(3, 3), l_shape());
    assert!(matches!(
        LevelSnapshot::from_parts(size, floor_cells(size), vec![out_of_bounds], Vec::new()),
        Err(LevelError::EntityOutOfBounds { .. })
    ));

    assert!(matches!(
        LevelSnapshot::from_parts(
            size,
            floor_cells(size),
            vec![first],
            vec![Decorator::key_locker("lock", "a", "missing")]
        ),
        Err(LevelError::MissingDecoratorEntity { .. })
    ));
}

#[test]
fn malformed_domain_json_is_rejected() {
    let disconnected = r#"{"width":2,"height":2,"occupied_mask":9}"#;
    assert!(
        serde_json::from_str::<Shape>(disconnected)
            .unwrap_err()
            .to_string()
            .contains("not connected")
    );

    let entity = PlaceableEntity::blind(
        "blind",
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(1, vec![BlindTile::empty(1).unwrap()]).unwrap(),
    )
    .unwrap();
    let size = GridSize::new(2, 2).unwrap();
    let snapshot =
        LevelSnapshot::from_parts(size, floor_cells(size), vec![entity], Vec::new()).unwrap();
    let mut malformed = serde_json::to_value(snapshot).unwrap();
    malformed["entities"][0]["kind"]["Blind"]["pixels_per_cell"] = serde_json::Value::from(33);
    assert!(
        serde_json::from_value::<LevelSnapshot>(malformed)
            .unwrap_err()
            .to_string()
            .contains("pixels-per-cell")
    );

    let valid = block("same", GridPoint::new(0, 0), Shape::new(1, 1, 1).unwrap());
    let snapshot =
        LevelSnapshot::from_parts(size, floor_cells(size), vec![valid.clone()], Vec::new())
            .unwrap();
    let mut duplicate = serde_json::to_value(snapshot).unwrap();
    duplicate["entities"] = serde_json::Value::Array(vec![
        serde_json::to_value(&valid).unwrap(),
        serde_json::to_value(valid).unwrap(),
    ]);
    assert!(
        serde_json::from_value::<LevelSnapshot>(duplicate)
            .unwrap_err()
            .to_string()
            .contains("occurs more than once")
    );
}

#[test]
fn collect_layers_preserve_order_and_values() {
    let layers = vec![
        CollectLayer::new(7, None, CollectCapacity::Unlimited, false),
        CollectLayer::new(2, Some(3), CollectCapacity::Finite(12), true),
    ];
    let block = Block::new(layers.clone());

    assert_eq!(block.collect_layers(), layers);
    assert_eq!(block.collect_layers()[0].radius(), None);
    assert_eq!(
        block.collect_layers()[1].capacity(),
        CollectCapacity::Finite(12)
    );
    assert!(block.collect_layers()[1].is_locked());
}
