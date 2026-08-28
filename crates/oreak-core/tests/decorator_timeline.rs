use oreak_core::{
    ApplyOutcome, Blind, BlindTile, Block, CellKind, CommandEnvelope, CommandMetadata, Decorator,
    DecoratorId, DecoratorKind, DecoratorRole, DirectionMode, EntityId, GridPoint, GridSize,
    LevelCommand, LevelError, LevelSnapshot, LevelTimeline, PlaceableEntity, Shape, TimelineError,
};

fn block(id: &str, x: u16) -> PlaceableEntity {
    PlaceableEntity::block(
        id,
        GridPoint::new(x, 0),
        Shape::new(1, 1, 1).unwrap(),
        Block::default(),
    )
    .unwrap()
}

fn blind(id: &str, x: u16) -> PlaceableEntity {
    PlaceableEntity::blind(
        id,
        GridPoint::new(x, 0),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(1, vec![BlindTile::empty(1).unwrap()]).unwrap(),
    )
    .unwrap()
}

fn snapshot(entities: Vec<PlaceableEntity>, decorators: Vec<Decorator>) -> LevelSnapshot {
    let size = GridSize::new(12, 4).unwrap();
    LevelSnapshot::from_parts(
        size,
        vec![CellKind::Floor; size.cell_count()],
        entities,
        decorators,
    )
    .unwrap()
}

fn command(id: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, "alice", 100), command)
}

#[test]
fn ice_count_uses_a_stable_zero_count_tombstone() {
    let mut timeline = LevelTimeline::new(snapshot(vec![block("a", 0)], Vec::new())).unwrap();
    timeline
        .apply(command(
            "set",
            LevelCommand::SetIce {
                decorator_id: DecoratorId::from("ice-a"),
                entity_id: EntityId::from("a"),
                blocking_count: 3,
            },
        ))
        .unwrap();
    assert_eq!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("ice-a"))
            .unwrap()
            .ice_blocking_count(),
        Some(3)
    );

    timeline
        .apply(command(
            "off",
            LevelCommand::ToggleIce {
                decorator_id: DecoratorId::from("ice-a"),
                entity_id: EntityId::from("a"),
            },
        ))
        .unwrap();
    let tombstone = timeline
        .snapshot()
        .decorator(&DecoratorId::from("ice-a"))
        .unwrap();
    assert_eq!(tombstone.id().as_str(), "ice-a");
    assert_eq!(tombstone.ice_blocking_count(), Some(0));

    timeline
        .apply(command(
            "on",
            LevelCommand::ToggleIce {
                decorator_id: DecoratorId::from("ice-a"),
                entity_id: EntityId::from("a"),
            },
        ))
        .unwrap();
    assert_eq!(
        timeline.snapshot().ice_blocking_count(&EntityId::from("a")),
        1
    );
    assert_eq!(
        timeline
            .blame_decorator(&DecoratorId::from("ice-a"))
            .unwrap()
            .command_id
            .as_str(),
        "on"
    );

    timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 200))
        .unwrap();
    assert_eq!(
        timeline.snapshot().ice_blocking_count(&EntityId::from("a")),
        0
    );
}

#[test]
fn direction_cycles_horizontal_vertical_and_disabled() {
    let mut timeline = LevelTimeline::new(snapshot(vec![block("a", 0)], Vec::new())).unwrap();
    for (id, expected) in [
        ("horizontal", DirectionMode::Horizontal),
        ("vertical", DirectionMode::Vertical),
        ("disabled", DirectionMode::Disabled),
    ] {
        timeline
            .apply(command(
                id,
                LevelCommand::CycleDirection {
                    decorator_id: DecoratorId::from("direction-a"),
                    entity_id: EntityId::from("a"),
                },
            ))
            .unwrap();
        assert_eq!(
            timeline.snapshot().direction_mode(&EntityId::from("a")),
            expected
        );
    }
    assert!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("direction-a"))
            .is_none()
    );

    timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 200))
        .unwrap();
    assert_eq!(
        timeline.snapshot().direction_mode(&EntityId::from("a")),
        DirectionMode::Vertical
    );

    let event_count = timeline.events().len();
    let outcome = timeline
        .apply(command(
            "same",
            LevelCommand::SetDirection {
                decorator_id: DecoratorId::from("direction-a"),
                entity_id: EntityId::from("a"),
                mode: DirectionMode::Vertical,
            },
        ))
        .unwrap();
    assert!(matches!(outcome, ApplyOutcome::NoChange { .. }));
    assert_eq!(timeline.events().len(), event_count);
}

#[test]
fn key_locker_requires_distinct_uniquely_owned_block_endpoints() {
    let mut timeline = LevelTimeline::new(snapshot(
        vec![
            block("key-a", 0),
            block("key-b", 2),
            block("lock-a", 4),
            block("lock-b", 6),
            blind("blind", 8),
        ],
        Vec::new(),
    ))
    .unwrap();
    timeline
        .apply(command(
            "assign",
            LevelCommand::AssignKeyLocker {
                decorator_id: DecoratorId::from("relation-a"),
                key_entity_id: EntityId::from("key-a"),
                lock_entity_id: EntityId::from("lock-a"),
            },
        ))
        .unwrap();

    let error = timeline
        .apply(command(
            "reuse-key",
            LevelCommand::AssignKeyLocker {
                decorator_id: DecoratorId::from("relation-b"),
                key_entity_id: EntityId::from("key-a"),
                lock_entity_id: EntityId::from("lock-b"),
            },
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        TimelineError::InvalidLevel(LevelError::DecoratorOwnershipConflict {
            role: DecoratorRole::Key,
            ..
        })
    ));

    let error = timeline
        .apply(command(
            "reuse-lock",
            LevelCommand::AssignKeyLocker {
                decorator_id: DecoratorId::from("relation-c"),
                key_entity_id: EntityId::from("key-b"),
                lock_entity_id: EntityId::from("lock-a"),
            },
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        TimelineError::InvalidLevel(LevelError::DecoratorOwnershipConflict {
            role: DecoratorRole::Lock,
            ..
        })
    ));

    assert!(matches!(
        timeline.apply(command(
            "same-endpoint",
            LevelCommand::AssignKeyLocker {
                decorator_id: DecoratorId::from("relation-d"),
                key_entity_id: EntityId::from("key-b"),
                lock_entity_id: EntityId::from("key-b"),
            }
        )),
        Err(TimelineError::InvalidLevel(
            LevelError::KeyLockerEndpointsMustDiffer { .. }
        ))
    ));
    assert!(matches!(
        timeline.apply(command(
            "blind-endpoint",
            LevelCommand::AssignKeyLocker {
                decorator_id: DecoratorId::from("relation-e"),
                key_entity_id: EntityId::from("blind"),
                lock_entity_id: EntityId::from("lock-b"),
            }
        )),
        Err(TimelineError::InvalidLevel(
            LevelError::KeyLockerEndpointNotBlock { .. }
        ))
    ));
    assert_eq!(timeline.events().len(), 1);

    timeline
        .apply(command(
            "remove",
            LevelCommand::RemoveKeyLocker {
                decorator_id: DecoratorId::from("relation-a"),
            },
        ))
        .unwrap();
    assert!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("relation-a"))
            .is_none()
    );
    timeline
        .undo_latest(CommandMetadata::new("undo-remove", "alice", 200))
        .unwrap();
    assert!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("relation-a"))
            .is_some()
    );
}

#[test]
fn deletion_cascades_owned_and_endpoint_decorators_and_undo_restores_them() {
    let entities = vec![block("key", 0), block("lock", 2)];
    let decorators = vec![
        Decorator::ice_with_blocking_count("ice-key", "key", 2),
        Decorator::direction_mode("direction-lock", "lock", DirectionMode::Horizontal).unwrap(),
        Decorator::key_locker("relation", "lock", "key"),
    ];
    let mut timeline = LevelTimeline::new(snapshot(entities, decorators)).unwrap();

    let ApplyOutcome::Applied(event) = timeline
        .apply(command(
            "delete-key",
            LevelCommand::DeleteEntity {
                entity_id: EntityId::from("key"),
            },
        ))
        .unwrap()
    else {
        panic!("expected deletion");
    };
    assert_eq!(event.changes.len(), 3);
    assert!(timeline.snapshot().entity(&EntityId::from("key")).is_none());
    assert!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("ice-key"))
            .is_none()
    );
    assert!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("relation"))
            .is_none()
    );
    assert!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("direction-lock"))
            .is_some()
    );

    timeline
        .undo_latest(CommandMetadata::new("undo-delete", "alice", 200))
        .unwrap();
    assert!(timeline.snapshot().entity(&EntityId::from("key")).is_some());
    assert_eq!(timeline.snapshot().decorators().len(), 3);
    assert_eq!(
        timeline
            .blame_decorator(&DecoratorId::from("relation"))
            .unwrap()
            .command_id
            .as_str(),
        "undo-delete"
    );
}

#[test]
fn malformed_and_duplicate_decorator_ownership_is_rejected() {
    let entities = vec![block("a", 0), block("b", 2)];
    let size = GridSize::new(12, 4).unwrap();
    let error = LevelSnapshot::from_parts(
        size,
        vec![CellKind::Floor; size.cell_count()],
        entities,
        vec![Decorator::ice("ice-a", "a"), Decorator::ice("ice-b", "a")],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        LevelError::DecoratorOwnershipConflict {
            role: DecoratorRole::Ice,
            ..
        }
    ));

    let snapshot = snapshot(vec![block("a", 0)], vec![Decorator::ice("legacy-ice", "a")]);
    let mut json = serde_json::to_value(snapshot).unwrap();
    json["decorators"][0]["kind"]["Ice"]
        .as_object_mut()
        .unwrap()
        .remove("blocking_count");
    let restored: LevelSnapshot = serde_json::from_value(json).unwrap();
    let DecoratorKind::Ice { blocking_count, .. } = restored.decorators()[0].kind() else {
        panic!("expected Ice");
    };
    assert_eq!(*blocking_count, 1);
}

#[test]
fn ice_and_direction_reject_blind_owners() {
    let mut timeline = LevelTimeline::new(snapshot(vec![blind("blind", 0)], Vec::new())).unwrap();
    for command_value in [
        LevelCommand::SetIce {
            decorator_id: DecoratorId::from("ice-blind"),
            entity_id: EntityId::from("blind"),
            blocking_count: 1,
        },
        LevelCommand::SetDirection {
            decorator_id: DecoratorId::from("direction-blind"),
            entity_id: EntityId::from("blind"),
            mode: DirectionMode::Horizontal,
        },
    ] {
        assert!(matches!(
            timeline.apply(command("blind-decorator", command_value)),
            Err(TimelineError::InvalidLevel(
                LevelError::DecoratorOwnerNotBlock { .. }
            ))
        ));
    }
    assert!(timeline.events().is_empty());
}

#[test]
fn later_owned_decorator_blocks_undo_of_entity_placement() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(6, 6).unwrap()).unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("place", "alice", 100),
            LevelCommand::PlaceEntity {
                entity: block("a", 0),
            },
        ))
        .unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("ice", "bob", 101),
            LevelCommand::SetIce {
                decorator_id: DecoratorId::from("ice-a"),
                entity_id: EntityId::from("a"),
                blocking_count: 1,
            },
        ))
        .unwrap();

    assert!(matches!(
        timeline.undo_latest(CommandMetadata::new("undo-place", "alice", 200)),
        Err(TimelineError::UndoConflict {
            blocked_sequences,
            ..
        }) if blocked_sequences == vec![1]
    ));
    assert!(timeline.snapshot().entity(&EntityId::from("a")).is_some());
    assert!(
        timeline
            .snapshot()
            .decorator(&DecoratorId::from("ice-a"))
            .is_some()
    );
}
