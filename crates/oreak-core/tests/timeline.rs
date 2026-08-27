use oreak_core::{
    ActorId, ApplyOutcome, CellKind, CommandEnvelope, CommandMetadata, GridError, GridPoint,
    LevelCommand, LevelSnapshot, LevelTimeline, TimelineError,
};

fn set_cell(
    command_id: &str,
    actor: &str,
    timestamp: i64,
    point: GridPoint,
    kind: CellKind,
) -> CommandEnvelope {
    CommandEnvelope::new(
        CommandMetadata::new(command_id, actor, timestamp),
        LevelCommand::SetCell { point, kind },
    )
}

#[test]
fn changed_cell_records_history_and_blame() {
    let point = GridPoint::new(2, 3);
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();

    let result = timeline
        .apply(set_cell("cmd-1", "alice", 100, point, CellKind::Wall))
        .unwrap();

    let ApplyOutcome::Applied(event) = result else {
        panic!("expected the cell change to be recorded");
    };
    assert_eq!(event.sequence, 1);
    assert_eq!(timeline.snapshot().cell(point), Ok(CellKind::Wall));
    assert_eq!(timeline.events().len(), 1);

    let blame = timeline.blame_cell(point).unwrap();
    assert_eq!(blame.actor, ActorId::from("alice"));
    assert_eq!(blame.command_id.as_str(), "cmd-1");
    assert_eq!(blame.sequence, 1);
}

#[test]
fn no_op_does_not_consume_history_or_command_id() {
    let point = GridPoint::new(1, 1);
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();

    let result = timeline
        .apply(set_cell("cmd-1", "alice", 100, point, CellKind::Floor))
        .unwrap();
    assert!(matches!(result, ApplyOutcome::NoChange { .. }));
    assert!(timeline.events().is_empty());
    assert!(timeline.blame_cell(point).is_none());

    let result = timeline
        .apply(set_cell("cmd-1", "alice", 101, point, CellKind::Wall))
        .unwrap();
    assert!(matches!(result, ApplyOutcome::Applied(_)));
}

#[test]
fn duplicate_accepted_command_id_is_rejected() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(set_cell(
            "cmd-1",
            "alice",
            100,
            GridPoint::new(0, 0),
            CellKind::Wall,
        ))
        .unwrap();

    let error = timeline
        .apply(set_cell(
            "cmd-1",
            "alice",
            101,
            GridPoint::new(1, 0),
            CellKind::Wall,
        ))
        .unwrap_err();

    assert!(matches!(error, TimelineError::DuplicateCommandId(_)));
    assert_eq!(timeline.events().len(), 1);
}

#[test]
fn undo_appends_a_compensating_event() {
    let point = GridPoint::new(4, 5);
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(set_cell("cmd-1", "alice", 100, point, CellKind::Wall))
        .unwrap();

    let undo = timeline
        .undo_latest(CommandMetadata::new("cmd-2", "alice", 200))
        .unwrap();

    assert_eq!(undo.sequence, 2);
    assert_eq!(undo.reverts_sequence, Some(1));
    assert_eq!(timeline.snapshot().cell(point), Ok(CellKind::Floor));
    assert_eq!(timeline.events().len(), 2);
    assert_eq!(timeline.history_for_cell(point).len(), 2);

    let blame = timeline.blame_cell(point).unwrap();
    assert_eq!(blame.command_id.as_str(), "cmd-2");
    assert_eq!(blame.reverts_sequence, Some(1));
}

#[test]
fn repeated_undo_walks_back_compatible_authored_events() {
    let first = GridPoint::new(0, 0);
    let second = GridPoint::new(1, 0);
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(set_cell("cmd-1", "alice", 100, first, CellKind::Wall))
        .unwrap();
    timeline
        .apply(set_cell("cmd-2", "alice", 101, second, CellKind::Wall))
        .unwrap();

    timeline
        .undo_latest(CommandMetadata::new("undo-2", "alice", 200))
        .unwrap();
    timeline
        .undo_latest(CommandMetadata::new("undo-1", "alice", 201))
        .unwrap();

    assert_eq!(timeline.snapshot().cell(first), Ok(CellKind::Floor));
    assert_eq!(timeline.snapshot().cell(second), Ok(CellKind::Floor));
    assert_eq!(timeline.events().len(), 4);
}

#[test]
fn collaborator_change_blocks_undo_of_the_overwritten_event() {
    let point = GridPoint::new(0, 0);
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(set_cell("alice-1", "alice", 100, point, CellKind::Wall))
        .unwrap();
    timeline
        .apply(set_cell("bob-1", "bob", 101, point, CellKind::Floor))
        .unwrap();

    let error = timeline
        .undo_latest(CommandMetadata::new("alice-undo", "alice", 200))
        .unwrap_err();

    assert_eq!(
        error,
        TimelineError::UndoConflict {
            actor: ActorId::from("alice"),
            blocked_sequences: vec![1],
        }
    );
    assert_eq!(timeline.snapshot().cell(point), Ok(CellKind::Floor));
}

#[test]
fn reverted_intervening_change_does_not_permanently_block_earlier_undo() {
    let point = GridPoint::new(0, 0);
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(set_cell("cmd-1", "alice", 100, point, CellKind::Wall))
        .unwrap();
    timeline
        .apply(set_cell("cmd-2", "alice", 101, point, CellKind::Floor))
        .unwrap();

    timeline
        .undo_latest(CommandMetadata::new("undo-2", "alice", 200))
        .unwrap();
    timeline
        .undo_latest(CommandMetadata::new("undo-1", "alice", 201))
        .unwrap();

    assert_eq!(timeline.snapshot().cell(point), Ok(CellKind::Floor));
    assert_eq!(timeline.events().len(), 4);
}

#[test]
fn out_of_bounds_command_is_rejected_without_history() {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();

    let error = timeline
        .apply(set_cell(
            "cmd-1",
            "alice",
            100,
            GridPoint::new(8, 0),
            CellKind::Wall,
        ))
        .unwrap_err();

    assert!(matches!(
        error,
        TimelineError::InvalidGrid(GridError::PointOutOfBounds { .. })
    ));
    assert!(timeline.events().is_empty());
}

#[test]
fn snapshot_hash_is_deterministic_and_content_sensitive() {
    let point = GridPoint::new(3, 3);
    let snapshot = LevelSnapshot::new(8, 8).unwrap();
    let expected = snapshot.content_hash();

    assert_eq!(snapshot.content_hash(), expected);

    let mut timeline = LevelTimeline::new(snapshot).unwrap();
    timeline
        .apply(set_cell("cmd-1", "alice", 100, point, CellKind::Wall))
        .unwrap();
    assert_ne!(timeline.snapshot().content_hash(), expected);
}
