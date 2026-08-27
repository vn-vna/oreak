use oreak_core::{
    ActorId, ApplyOutcome, Block, CellKind, CommandEnvelope, CommandMetadata, Decorator, EntityId,
    GridPoint, GridSize, LevelCommand, LevelError, LevelSnapshot, LevelTimeline, PlaceableEntity,
    Shape, ShapeCell, TimelineError,
};

fn l_shape() -> Shape {
    Shape::from_cells(&[
        ShapeCell::new(0, 0),
        ShapeCell::new(1, 0),
        ShapeCell::new(0, 1),
    ])
    .unwrap()
}

fn entity(id: &str, origin: GridPoint, shape: Shape) -> PlaceableEntity {
    PlaceableEntity::block(id, origin, shape, Block::default()).unwrap()
}

fn command(id: &str, actor: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, actor, 100), command)
}

fn place(id: &str, actor: &str, entity: PlaceableEntity) -> CommandEnvelope {
    command(id, actor, LevelCommand::PlaceEntity { entity })
}

#[test]
fn placement_is_ordered_and_records_entity_blame() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(place(
            "place-b",
            "alice",
            entity("b", GridPoint::new(4, 4), l_shape()),
        ))
        .unwrap();
    let result = timeline
        .apply(place(
            "place-a",
            "bob",
            entity("a", GridPoint::new(0, 0), Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap();

    assert!(matches!(result, ApplyOutcome::Applied(_)));
    assert_eq!(timeline.snapshot().entities()[0].id().as_str(), "a");
    assert_eq!(timeline.snapshot().entities()[1].id().as_str(), "b");
    let blame = timeline.blame_entity(&EntityId::from("a")).unwrap();
    assert_eq!(blame.actor, ActorId::from("bob"));
    assert_eq!(blame.command_id.as_str(), "place-a");
    assert_eq!(timeline.history_for_entity(&EntityId::from("a")).len(), 1);
}

#[test]
fn invalid_placement_is_atomic_and_does_not_consume_command_id() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(5, 5).unwrap()).unwrap();
    timeline
        .apply(place(
            "first",
            "alice",
            entity("a", GridPoint::new(1, 1), l_shape()),
        ))
        .unwrap();
    let before_hash = timeline.snapshot().content_hash();

    let error = timeline
        .apply(place(
            "retry",
            "alice",
            entity("b", GridPoint::new(2, 1), Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        TimelineError::InvalidLevel(LevelError::EntityOverlap { .. })
    ));
    assert_eq!(timeline.events().len(), 1);
    assert_eq!(timeline.snapshot().content_hash(), before_hash);

    timeline
        .apply(place(
            "retry",
            "alice",
            entity("b", GridPoint::new(4, 4), Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap();
    assert_eq!(timeline.events().len(), 2);
}

#[test]
fn move_and_rotation_fail_without_partial_changes() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(6, 6).unwrap()).unwrap();
    timeline
        .apply(place(
            "place-a",
            "alice",
            entity("a", GridPoint::new(1, 1), Shape::new(2, 1, 0b11).unwrap()),
        ))
        .unwrap();
    timeline
        .apply(place(
            "place-b",
            "bob",
            entity("b", GridPoint::new(1, 2), Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap();
    let before_hash = timeline.snapshot().content_hash();

    assert!(matches!(
        timeline.apply(command(
            "rotate-a",
            "alice",
            LevelCommand::RotateEntityClockwise {
                entity_id: EntityId::from("a")
            }
        )),
        Err(TimelineError::InvalidLevel(
            LevelError::EntityOverlap { .. }
        ))
    ));
    assert!(matches!(
        timeline.apply(command(
            "move-a",
            "alice",
            LevelCommand::MoveEntity {
                entity_id: EntityId::from("a"),
                origin: GridPoint::new(5, 5)
            }
        )),
        Err(TimelineError::InvalidLevel(
            LevelError::EntityOutOfBounds { .. }
        ))
    ));
    assert_eq!(timeline.snapshot().content_hash(), before_hash);
    assert_eq!(timeline.events().len(), 2);
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("a"))
            .unwrap()
            .origin(),
        GridPoint::new(1, 1)
    );
}

#[test]
fn horizontal_flip_collision_is_atomic() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(6, 6).unwrap()).unwrap();
    timeline
        .apply(place(
            "place-a",
            "alice",
            entity("a", GridPoint::new(1, 1), l_shape()),
        ))
        .unwrap();
    timeline
        .apply(place(
            "place-b",
            "bob",
            entity("b", GridPoint::new(2, 2), Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap();
    let before_hash = timeline.snapshot().content_hash();

    assert!(matches!(
        timeline.apply(command(
            "flip-a",
            "alice",
            LevelCommand::FlipEntityHorizontal {
                entity_id: EntityId::from("a")
            }
        )),
        Err(TimelineError::InvalidLevel(
            LevelError::EntityOverlap { .. }
        ))
    ));
    assert_eq!(timeline.snapshot().content_hash(), before_hash);
    assert_eq!(timeline.events().len(), 2);
}

#[test]
fn transforms_preserve_ids_and_undo_restores_exact_entity() {
    let original = entity("a", GridPoint::new(2, 2), l_shape());
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(place("place", "alice", original.clone()))
        .unwrap();
    let original_hash = timeline.snapshot().content_hash();
    timeline
        .apply(command(
            "rotate",
            "alice",
            LevelCommand::RotateEntityClockwise {
                entity_id: EntityId::from("a"),
            },
        ))
        .unwrap();

    let rotated = timeline.snapshot().entity(&EntityId::from("a")).unwrap();
    assert_eq!(rotated.id().as_str(), "a");
    assert_ne!(rotated, &original);
    assert_ne!(timeline.snapshot().content_hash(), original_hash);
    let undo = timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 200))
        .unwrap();
    assert_eq!(undo.reverts_sequence, Some(2));
    assert_eq!(
        timeline.snapshot().entity(&EntityId::from("a")),
        Some(&original)
    );
    assert_eq!(timeline.snapshot().content_hash(), original_hash);

    timeline
        .apply(command(
            "flip",
            "alice",
            LevelCommand::FlipEntityHorizontal {
                entity_id: EntityId::from("a"),
            },
        ))
        .unwrap();
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("a"))
            .unwrap()
            .id()
            .as_str(),
        "a"
    );
}

#[test]
fn transform_no_ops_create_no_history_or_consumed_id() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(5, 5).unwrap()).unwrap();
    timeline
        .apply(place(
            "place",
            "alice",
            entity("dot", GridPoint::new(1, 1), Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap();

    let outcome = timeline
        .apply(command(
            "same-id",
            "alice",
            LevelCommand::RotateEntityClockwise {
                entity_id: EntityId::from("dot"),
            },
        ))
        .unwrap();
    assert!(matches!(outcome, ApplyOutcome::NoChange { .. }));
    assert_eq!(timeline.events().len(), 1);

    timeline
        .apply(command(
            "same-id",
            "alice",
            LevelCommand::MoveEntity {
                entity_id: EntityId::from("dot"),
                origin: GridPoint::new(2, 2),
            },
        ))
        .unwrap();
    assert_eq!(timeline.events().len(), 2);
}

#[test]
fn delete_and_undo_are_compensating_events() {
    let original = entity("a", GridPoint::new(1, 1), l_shape());
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(place("place", "alice", original.clone()))
        .unwrap();
    timeline
        .apply(command(
            "delete",
            "alice",
            LevelCommand::DeleteEntity {
                entity_id: EntityId::from("a"),
            },
        ))
        .unwrap();
    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_none());

    let undo = timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 200))
        .unwrap();
    assert_eq!(undo.reverts_sequence, Some(2));
    assert_eq!(
        timeline.snapshot().entity(&EntityId::from("a")),
        Some(&original)
    );
    assert_eq!(timeline.history_for_entity(&EntityId::from("a")).len(), 3);
    assert_eq!(
        timeline
            .blame_entity(&EntityId::from("a"))
            .unwrap()
            .command_id
            .as_str(),
        "undo"
    );
}

#[test]
fn later_entity_edits_and_spatial_reuse_block_undo() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(place(
            "alice-place",
            "alice",
            entity("a", GridPoint::new(1, 1), l_shape()),
        ))
        .unwrap();
    timeline
        .apply(command(
            "bob-move",
            "bob",
            LevelCommand::MoveEntity {
                entity_id: EntityId::from("a"),
                origin: GridPoint::new(4, 4),
            },
        ))
        .unwrap();

    assert_eq!(
        timeline
            .undo_latest(CommandMetadata::new("alice-undo", "alice", 200))
            .unwrap_err(),
        TimelineError::UndoConflict {
            actor: ActorId::from("alice"),
            blocked_sequences: vec![1],
        }
    );

    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(place(
            "place-a",
            "alice",
            entity("a", GridPoint::new(1, 1), l_shape()),
        ))
        .unwrap();
    timeline
        .apply(command(
            "delete-a",
            "alice",
            LevelCommand::DeleteEntity {
                entity_id: EntityId::from("a"),
            },
        ))
        .unwrap();
    timeline
        .apply(place(
            "place-b",
            "bob",
            entity("b", GridPoint::new(1, 1), Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap();

    assert!(matches!(
        timeline.undo_latest(CommandMetadata::new("undo-delete", "alice", 200)),
        Err(TimelineError::UndoConflict { .. })
    ));
    assert_eq!(timeline.events().len(), 3);
}

#[test]
fn walls_cannot_be_created_under_entities() {
    let point = GridPoint::new(2, 2);
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(5, 5).unwrap()).unwrap();
    timeline
        .apply(place(
            "place",
            "alice",
            entity("dot", point, Shape::new(1, 1, 1).unwrap()),
        ))
        .unwrap();
    let before_hash = timeline.snapshot().content_hash();

    assert!(matches!(
        timeline.apply(command(
            "wall",
            "alice",
            LevelCommand::SetCell {
                point,
                kind: CellKind::Wall
            }
        )),
        Err(TimelineError::InvalidLevel(
            LevelError::CellOccupiedByEntity { .. }
        ))
    ));
    assert_eq!(timeline.snapshot().content_hash(), before_hash);
    assert_eq!(timeline.events().len(), 1);
}

#[test]
fn referenced_entities_and_decorators_are_deleted_atomically() {
    let size = GridSize::new(5, 5).unwrap();
    let entity = entity("a", GridPoint::new(1, 1), Shape::new(1, 1, 1).unwrap());
    let snapshot = LevelSnapshot::from_parts(
        size,
        vec![CellKind::Floor; size.cell_count()],
        vec![entity],
        vec![Decorator::ice("ice", "a")],
    )
    .unwrap();
    let mut timeline = LevelTimeline::new(snapshot).unwrap();

    let ApplyOutcome::Applied(event) = timeline
        .apply(command(
            "delete",
            "alice",
            LevelCommand::DeleteEntity {
                entity_id: EntityId::from("a"),
            },
        ))
        .unwrap()
    else {
        panic!("expected cascaded deletion");
    };
    assert_eq!(event.changes.len(), 2);
    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_none());
    assert!(timeline.snapshot().decorators().is_empty());

    timeline
        .undo_latest(CommandMetadata::new("undo-delete", "alice", 200))
        .unwrap();
    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_some());
    assert_eq!(timeline.snapshot().decorators().len(), 1);
}
