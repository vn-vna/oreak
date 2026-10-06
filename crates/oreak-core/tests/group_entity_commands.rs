use oreak_core::{
    Block, CollectCapacity, CollectLayer, CommandEnvelope, CommandMetadata, EntityId, GridPoint,
    LevelCommand, LevelError, LevelSnapshot, LevelTimeline, PlaceableEntity, PlaceableEntityKind,
    Shape, TimelineError,
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
                entities: vec![
                    a.moved_to(GridPoint::new(1, 0)),
                    b.moved_to(GridPoint::new(0, 0)),
                ],
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
