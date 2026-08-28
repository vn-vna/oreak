use oreak_core::{
    Block, CommandEnvelope, CommandMetadata, EntityId, EntityMove, GridPoint, LevelCommand,
    LevelError, LevelSnapshot, LevelTimeline, PlaceableEntity, Shape, TimelineError,
};

fn command(id: &str, actor: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, actor, 100), command)
}

fn dot(id: &str, x: u16, y: u16) -> PlaceableEntity {
    PlaceableEntity::block(
        id,
        GridPoint::new(x, y),
        Shape::new(1, 1, 1).unwrap(),
        Block::default(),
    )
    .unwrap()
}

fn placed_pair() -> LevelTimeline {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(5, 4).unwrap()).unwrap();
    for (command_id, entity) in [("place-a", dot("a", 1, 1)), ("place-b", dot("b", 2, 1))] {
        timeline
            .apply(command(
                command_id,
                "alice",
                LevelCommand::PlaceEntity { entity },
            ))
            .unwrap();
    }
    timeline
}

#[test]
fn grouped_move_validates_the_final_arrangement_atomically() {
    let mut timeline = placed_pair();
    let outcome = timeline
        .apply(command(
            "move-pair",
            "alice",
            LevelCommand::MoveEntities {
                moves: vec![
                    EntityMove::new("a", GridPoint::new(2, 1)),
                    EntityMove::new("b", GridPoint::new(3, 1)),
                ],
            },
        ))
        .unwrap();

    let oreak_core::ApplyOutcome::Applied(event) = outcome else {
        panic!("grouped move should be recorded");
    };
    assert_eq!(event.changes.len(), 2);
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("a"))
            .unwrap()
            .origin(),
        GridPoint::new(2, 1)
    );
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("b"))
            .unwrap()
            .origin(),
        GridPoint::new(3, 1)
    );
}

#[test]
fn invalid_grouped_move_leaves_every_entity_unchanged() {
    let mut timeline = placed_pair();
    let before = timeline.snapshot().clone();
    let event_count = timeline.events().len();

    let error = timeline
        .apply(command(
            "move-pair",
            "alice",
            LevelCommand::MoveEntities {
                moves: vec![
                    EntityMove::new("a", GridPoint::new(2, 1)),
                    EntityMove::new("b", GridPoint::new(5, 1)),
                ],
            },
        ))
        .unwrap_err();

    assert!(matches!(
        error,
        TimelineError::InvalidLevel(LevelError::EntityOutOfBounds { .. })
    ));
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.events().len(), event_count);
}

#[test]
fn duplicate_grouped_move_ids_are_rejected() {
    let mut timeline = placed_pair();
    let error = timeline
        .apply(command(
            "move-pair",
            "alice",
            LevelCommand::MoveEntities {
                moves: vec![
                    EntityMove::new("a", GridPoint::new(0, 0)),
                    EntityMove::new("a", GridPoint::new(0, 1)),
                ],
            },
        ))
        .unwrap_err();

    assert_eq!(
        error,
        TimelineError::InvalidLevel(LevelError::DuplicateEntityMove(EntityId::from("a")))
    );
}

#[test]
fn grouped_move_undo_restores_all_origins() {
    let mut timeline = placed_pair();
    timeline
        .apply(command(
            "move-pair",
            "alice",
            LevelCommand::MoveEntities {
                moves: vec![
                    EntityMove::new("a", GridPoint::new(2, 2)),
                    EntityMove::new("b", GridPoint::new(3, 2)),
                ],
            },
        ))
        .unwrap();

    let undo = timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 200))
        .unwrap();

    assert_eq!(undo.reverts_sequence, Some(3));
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("a"))
            .unwrap()
            .origin(),
        GridPoint::new(1, 1)
    );
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("b"))
            .unwrap()
            .origin(),
        GridPoint::new(2, 1)
    );
    assert_eq!(timeline.history_for_entity(&EntityId::from("a")).len(), 3);
    assert_eq!(timeline.history_for_entity(&EntityId::from("b")).len(), 3);
}
