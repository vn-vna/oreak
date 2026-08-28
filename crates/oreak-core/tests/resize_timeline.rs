use oreak_core::{
    ActorId, Block, CellKind, CommandEnvelope, CommandMetadata, EntityId, GridAnchor, GridPoint,
    GridSize, LevelCommand, LevelError, LevelSnapshot, LevelTimeline, PlaceableEntity, Shape,
    TimelineError,
};

fn command(id: &str, actor: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, actor, 100), command)
}

fn dot(id: &str, origin: GridPoint) -> PlaceableEntity {
    PlaceableEntity::block(id, origin, Shape::new(1, 1, 1).unwrap(), Block::default()).unwrap()
}

#[test]
fn anchored_resize_translates_cells_and_entities() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 4).unwrap()).unwrap();
    timeline
        .apply(command(
            "wall",
            "alice",
            LevelCommand::SetCell {
                point: GridPoint::new(0, 0),
                kind: CellKind::Wall,
            },
        ))
        .unwrap();
    timeline
        .apply(command(
            "place",
            "alice",
            LevelCommand::PlaceEntity {
                entity: dot("dot", GridPoint::new(1, 1)),
            },
        ))
        .unwrap();

    timeline
        .apply(command(
            "resize",
            "alice",
            LevelCommand::ResizeGrid {
                size: GridSize::new(6, 6).unwrap(),
                anchor: GridAnchor::TopRight,
            },
        ))
        .unwrap();

    assert_eq!(timeline.snapshot().size(), GridSize::new(6, 6).unwrap());
    assert_eq!(
        timeline.snapshot().cell(GridPoint::new(2, 2)),
        Ok(CellKind::Wall)
    );
    assert_eq!(
        timeline
            .snapshot()
            .entity(&EntityId::from("dot"))
            .unwrap()
            .origin(),
        GridPoint::new(3, 3)
    );
}

#[test]
fn shrinking_rejects_clipped_content_without_partial_changes() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 4).unwrap()).unwrap();
    timeline
        .apply(command(
            "wall",
            "alice",
            LevelCommand::SetCell {
                point: GridPoint::new(3, 3),
                kind: CellKind::Wall,
            },
        ))
        .unwrap();
    let before = timeline.snapshot().clone();

    let error = timeline
        .apply(command(
            "resize",
            "alice",
            LevelCommand::ResizeGrid {
                size: GridSize::new(3, 3).unwrap(),
                anchor: GridAnchor::BottomLeft,
            },
        ))
        .unwrap_err();

    assert_eq!(
        error,
        TimelineError::InvalidLevel(LevelError::ResizeWouldClipCell {
            point: GridPoint::new(3, 3)
        })
    );
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.events().len(), 1);
}

#[test]
fn resize_undo_restores_the_exact_snapshot() {
    let original = LevelSnapshot::new(5, 4).unwrap();
    let original_hash = original.content_hash();
    let mut timeline = LevelTimeline::new(original).unwrap();
    timeline
        .apply(command(
            "resize",
            "alice",
            LevelCommand::ResizeGrid {
                size: GridSize::new(8, 7).unwrap(),
                anchor: GridAnchor::Center,
            },
        ))
        .unwrap();

    let undo = timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 200))
        .unwrap();

    assert_eq!(undo.reverts_sequence, Some(1));
    assert_eq!(timeline.snapshot().size(), GridSize::new(5, 4).unwrap());
    assert_eq!(timeline.snapshot().content_hash(), original_hash);
}

#[test]
fn later_collaborator_edit_blocks_resize_undo() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 4).unwrap()).unwrap();
    timeline
        .apply(command(
            "resize",
            "alice",
            LevelCommand::ResizeGrid {
                size: GridSize::new(5, 5).unwrap(),
                anchor: GridAnchor::BottomLeft,
            },
        ))
        .unwrap();
    timeline
        .apply(command(
            "bob-wall",
            "bob",
            LevelCommand::SetCell {
                point: GridPoint::new(4, 4),
                kind: CellKind::Wall,
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
    assert_eq!(timeline.snapshot().size(), GridSize::new(5, 5).unwrap());
}
