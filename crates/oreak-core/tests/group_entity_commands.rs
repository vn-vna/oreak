use oreak_core::{
    Block, CollectCapacity, CollectLayer, CommandEnvelope, CommandMetadata, EntityError, EntityId,
    EntityTransform, GridPoint, LevelCommand, LevelError, LevelSnapshot, LevelTimeline,
    PlaceableEntity, PlaceableEntityKind, Shape, TimelineError,
};

fn command(id: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, "alice", 100), command)
}

fn block(id: &str, x: u16, y: u16, capacity: CollectCapacity) -> PlaceableEntity {
    PlaceableEntity::block(
        id,
        GridPoint::new(x, y),
        Shape::new(1, 1, 1).unwrap(),
        Block::new(vec![CollectLayer::new(1, None, capacity, false)]),
    )
    .unwrap()
}

fn capacity_of(timeline: &LevelTimeline, id: &str) -> CollectCapacity {
    let entity = timeline.snapshot().entity(&EntityId::from(id)).unwrap();
    let PlaceableEntityKind::Block(block) = entity.kind() else {
        panic!("expected Block");
    };
    block.collect_layers()[0].capacity()
}

#[test]
fn place_entities_is_atomic_and_undoes_as_one_event() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    timeline
        .apply(command(
            "place-pair",
            LevelCommand::PlaceEntities {
                entities: vec![
                    block("a", 0, 0, CollectCapacity::Unlimited),
                    block("b", 1, 0, CollectCapacity::Unlimited),
                ],
            },
        ))
        .unwrap();

    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_some());
    assert!(timeline.snapshot().entity(&EntityId::from("b")).is_some());
    timeline
        .undo_latest(CommandMetadata::new("undo-place", "alice", 200))
        .unwrap();
    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_none());
    assert!(timeline.snapshot().entity(&EntityId::from("b")).is_none());
}

#[test]
fn transform_entities_validates_the_final_arrangement_and_undoes_together() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    let a = block("a", 0, 0, CollectCapacity::Unlimited);
    let b = block("b", 1, 0, CollectCapacity::Unlimited);
    timeline
        .apply(command(
            "place-pair",
            LevelCommand::PlaceEntities {
                entities: vec![a.clone(), b.clone()],
            },
        ))
        .unwrap();

    timeline
        .apply(command(
            "swap-pair",
            LevelCommand::TransformEntities {
                transforms: vec![
                    EntityTransform::new("a").moved_to(GridPoint::new(1, 0)),
                    EntityTransform::new("b").moved_to(GridPoint::new(0, 0)),
                ],
                entities: Vec::new(),
            },
        ))
        .unwrap();
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("a"))
            .unwrap()
            .origin(),
        GridPoint::new(1, 0)
    );
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("b"))
            .unwrap()
            .origin(),
        GridPoint::new(0, 0)
    );

    timeline
        .undo_latest(CommandMetadata::new("undo-swap", "alice", 200))
        .unwrap();
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("a"))
            .unwrap()
            .origin(),
        GridPoint::new(0, 0)
    );
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("b"))
            .unwrap()
            .origin(),
        GridPoint::new(1, 0)
    );
}

#[test]
fn delete_entities_removes_and_restores_the_group_in_one_undo() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    timeline
        .apply(command(
            "place-pair",
            LevelCommand::PlaceEntities {
                entities: vec![
                    block("a", 0, 0, CollectCapacity::Unlimited),
                    block("b", 1, 0, CollectCapacity::Unlimited),
                ],
            },
        ))
        .unwrap();
    timeline
        .apply(command(
            "delete-pair",
            LevelCommand::DeleteEntities {
                entity_ids: vec![EntityId::from("a"), EntityId::from("b")],
            },
        ))
        .unwrap();
    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_none());
    assert!(timeline.snapshot().entity(&EntityId::from("b")).is_none());

    timeline
        .undo_latest(CommandMetadata::new("undo-delete", "alice", 200))
        .unwrap();
    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_some());
    assert!(timeline.snapshot().entity(&EntityId::from("b")).is_some());
}

#[test]
fn delete_entities_restores_shared_decorators_after_group_undo() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    timeline
        .apply(command(
            "place-pair",
            LevelCommand::PlaceEntities {
                entities: vec![
                    block("key", 0, 0, CollectCapacity::Unlimited),
                    block("lock", 1, 0, CollectCapacity::Unlimited),
                ],
            },
        ))
        .unwrap();
    timeline
        .apply(command(
            "link-pair",
            LevelCommand::AssignKeyLocker {
                decorator_id: "link".into(),
                key_entity_id: "key".into(),
                lock_entity_id: "lock".into(),
            },
        ))
        .unwrap();
    timeline
        .apply(command(
            "delete-pair",
            LevelCommand::DeleteEntities {
                entity_ids: vec![EntityId::from("key"), EntityId::from("lock")],
            },
        ))
        .unwrap();
    assert!(timeline.snapshot().decorator(&"link".into()).is_none());

    timeline
        .undo_latest(CommandMetadata::new("undo-delete", "alice", 200))
        .unwrap();
    assert!(timeline.snapshot().decorator(&"link".into()).is_some());
}

#[test]
fn batch_capacity_edit_preserves_each_block_otherwise_and_undoes() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    timeline
        .apply(command(
            "place-pair",
            LevelCommand::PlaceEntities {
                entities: vec![
                    block("a", 0, 0, CollectCapacity::Finite(2)),
                    block("b", 1, 0, CollectCapacity::Finite(7)),
                ],
            },
        ))
        .unwrap();

    timeline
        .apply(command(
            "capacity-pair",
            LevelCommand::SetBlockLayerCapacity {
                entity_ids: vec![EntityId::from("a"), EntityId::from("b")],
                layer_index: 0,
                capacity: CollectCapacity::Finite(9),
            },
        ))
        .unwrap();
    assert_eq!(capacity_of(&timeline, "a"), CollectCapacity::Finite(9));
    assert_eq!(capacity_of(&timeline, "b"), CollectCapacity::Finite(9));

    timeline
        .undo_latest(CommandMetadata::new("undo-capacity", "alice", 200))
        .unwrap();
    assert_eq!(capacity_of(&timeline, "a"), CollectCapacity::Finite(2));
    assert_eq!(capacity_of(&timeline, "b"), CollectCapacity::Finite(7));
}

#[test]
fn semantic_transform_preserves_concurrent_capacity_edits() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    let entity = PlaceableEntity::block(
        "a",
        GridPoint::new(0, 0),
        Shape::new(2, 1, 0b11).unwrap(),
        Block::new(vec![CollectLayer::new(
            1,
            None,
            CollectCapacity::Finite(2),
            false,
        )]),
    )
    .unwrap();
    timeline
        .apply(command("place", LevelCommand::PlaceEntity { entity }))
        .unwrap();
    timeline
        .apply(command(
            "capacity",
            LevelCommand::SetBlockLayerCapacity {
                entity_ids: vec![EntityId::from("a")],
                layer_index: 0,
                capacity: CollectCapacity::Finite(9),
            },
        ))
        .unwrap();

    timeline
        .apply(command(
            "rotate-and-move",
            LevelCommand::TransformEntities {
                transforms: vec![
                    EntityTransform::new("a")
                        .moved_to(GridPoint::new(1, 0))
                        .rotated_clockwise(1),
                ],
                entities: Vec::new(),
            },
        ))
        .unwrap();
    let transformed = timeline.snapshot().entity(&EntityId::from("a")).unwrap();
    assert_eq!(transformed.origin(), GridPoint::new(1, 0));
    assert_eq!(transformed.shape(), Shape::new(1, 2, 0b11).unwrap());
    assert_eq!(capacity_of(&timeline, "a"), CollectCapacity::Finite(9));
}

#[test]
fn locked_collect_layers_reject_capacity_and_wholesale_changes() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    let entity = PlaceableEntity::block(
        "locked",
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        Block::new(vec![CollectLayer::new(
            1,
            None,
            CollectCapacity::Finite(2),
            true,
        )]),
    )
    .unwrap();
    timeline
        .apply(command("place", LevelCommand::PlaceEntity { entity }))
        .unwrap();
    let before = timeline.snapshot().clone();

    let capacity_error = timeline
        .apply(command(
            "locked-capacity",
            LevelCommand::SetBlockLayerCapacity {
                entity_ids: vec![EntityId::from("locked")],
                layer_index: 0,
                capacity: CollectCapacity::Finite(9),
            },
        ))
        .unwrap_err();
    assert!(matches!(
        capacity_error,
        TimelineError::InvalidLevel(LevelError::InvalidEntity(
            EntityError::CollectLayerLocked { .. }
        ))
    ));
    assert_eq!(timeline.snapshot(), &before);

    let layers_error = timeline
        .apply(command(
            "remove-locked-layer",
            LevelCommand::SetBlockCollectLayers {
                entity_id: EntityId::from("locked"),
                layers: Vec::new(),
            },
        ))
        .unwrap_err();
    assert!(matches!(
        layers_error,
        TimelineError::InvalidLevel(LevelError::InvalidEntity(
            EntityError::CollectLayerLocked { .. }
        ))
    ));
    assert_eq!(timeline.snapshot(), &before);
}

#[test]
fn collect_layer_replacement_rejects_values_not_representable_by_legacy() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 3).unwrap()).unwrap();
    timeline
        .apply(command(
            "place",
            LevelCommand::PlaceEntity {
                entity: block("a", 0, 0, CollectCapacity::Unlimited),
            },
        ))
        .unwrap();
    let before = timeline.snapshot().clone();

    let color_error = timeline
        .apply(command(
            "invalid-color",
            LevelCommand::SetBlockCollectLayers {
                entity_id: EntityId::from("a"),
                layers: vec![CollectLayer::new(
                    16,
                    None,
                    CollectCapacity::Unlimited,
                    false,
                )],
            },
        ))
        .unwrap_err();
    assert!(matches!(
        color_error,
        TimelineError::InvalidLevel(LevelError::InvalidEntity(
            EntityError::InvalidCollectLayerColor(16)
        ))
    ));
    assert_eq!(timeline.snapshot(), &before);

    let capacity_error = timeline
        .apply(command(
            "invalid-capacity",
            LevelCommand::SetBlockCollectLayers {
                entity_id: EntityId::from("a"),
                layers: vec![CollectLayer::new(
                    1,
                    None,
                    CollectCapacity::Finite(i32::MAX as u32 + 1),
                    false,
                )],
            },
        ))
        .unwrap_err();
    assert!(matches!(
        capacity_error,
        TimelineError::InvalidLevel(LevelError::InvalidEntity(
            EntityError::CollectCapacityTooLarge(_)
        ))
    ));
    assert_eq!(timeline.snapshot(), &before);
}

#[test]
fn invalid_batch_placement_leaves_snapshot_unchanged() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(3, 3).unwrap()).unwrap();
    let before = timeline.snapshot().clone();
    let error = timeline
        .apply(command(
            "invalid-place",
            LevelCommand::PlaceEntities {
                entities: vec![
                    block("a", 0, 0, CollectCapacity::Unlimited),
                    block("b", 0, 0, CollectCapacity::Unlimited),
                ],
            },
        ))
        .unwrap_err();

    assert!(matches!(
        error,
        TimelineError::InvalidLevel(LevelError::EntityOverlap { .. })
    ));
    assert_eq!(timeline.snapshot(), &before);
}
