use oreak_core::*;

fn fixture() -> (LevelSnapshot, DistributionRequest) {
    let shape = Shape::new(1, 1, 1).unwrap();
    let pool = PlaceableEntity::blind(
        "pool",
        GridPoint::new(0, 0),
        shape,
        Blind::new(8, vec![BlindTile::from_colors(8, vec![1; 64]).unwrap()]).unwrap(),
    )
    .unwrap();
    let block = PlaceableEntity::block(
        "block",
        GridPoint::new(1, 0),
        shape,
        Block::new(vec![
            CollectLayer::new(1, Some(5), CollectCapacity::Finite(6), true),
            CollectLayer::new(1, Some(2), CollectCapacity::Unlimited, false),
            CollectLayer::new(1, None, CollectCapacity::Finite(99), false),
            CollectLayer::new(2, None, CollectCapacity::Finite(17), false),
        ]),
    )
    .unwrap();
    let size = GridSize::new(4, 4).unwrap();
    let snapshot =
        LevelSnapshot::from_parts(size, vec![CellKind::Floor; 16], vec![pool, block], vec![])
            .unwrap();
    let request = DistributionRequest {
        pools: vec![DistributionPoolSelection {
            entity_id: "pool".into(),
            group_ids: vec![0],
        }],
        layers: vec![0, 1, 2]
            .into_iter()
            .map(|layer_index| DistributionLayerSelection {
                entity_id: "block".into(),
                layer_index,
                weight: if layer_index == 2 { 2 } else { 1 },
            })
            .collect(),
    };
    (snapshot, request)
}
fn envelope(id: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, "alice", 1), command)
}
fn command(snapshot: &LevelSnapshot, request: DistributionRequest) -> LevelCommand {
    LevelCommand::ApplyDistribution {
        request,
        expected_entities: snapshot.entities().to_vec(),
    }
}
#[test]
fn allocation_is_one_exact_undoable_command_preserving_locks_and_other_rows() {
    let (snapshot, request) = fixture();
    let mut timeline = LevelTimeline::new(snapshot.clone()).unwrap();
    assert!(matches!(
        timeline
            .apply(envelope("distribution", command(&snapshot, request)))
            .unwrap(),
        ApplyOutcome::Applied(_)
    ));
    let PlaceableEntityKind::Block(block) =
        timeline.snapshot().entity(&"block".into()).unwrap().kind()
    else {
        panic!()
    };
    let capacities: Vec<_> = block
        .collect_layers()
        .iter()
        .map(|layer| layer.capacity())
        .collect();
    assert_eq!(
        capacities,
        vec![
            CollectCapacity::Finite(6),
            CollectCapacity::Finite(10),
            CollectCapacity::Finite(20),
            CollectCapacity::Finite(17)
        ]
    );
    assert_eq!(block.collect_layers()[1].radius(), Some(2));
    assert!(block.collect_layers()[0].is_locked());
    assert_eq!(timeline.events().len(), 1);
    timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 2))
        .unwrap();
    assert_eq!(timeline.snapshot(), &snapshot);
}
#[test]
fn missing_duplicate_and_stale_guards_reject_without_mutation() {
    let (snapshot, request) = fixture();
    let mut timeline = LevelTimeline::new(snapshot.clone()).unwrap();
    for expected_entities in [
        vec![],
        vec![
            snapshot.entities()[0].clone(),
            snapshot.entities()[0].clone(),
        ],
    ] {
        assert!(
            timeline
                .apply(envelope(
                    "invalid",
                    LevelCommand::ApplyDistribution {
                        request: request.clone(),
                        expected_entities
                    }
                ))
                .is_err()
        );
        assert_eq!(timeline.snapshot(), &snapshot);
        assert!(timeline.events().is_empty());
    }
    let stale = command(&snapshot, request);
    timeline
        .apply(envelope(
            "edit",
            LevelCommand::SetBlockLayerCapacity {
                entity_ids: vec!["block".into()],
                layer_index: 1,
                capacity: CollectCapacity::Finite(1),
            },
        ))
        .unwrap();
    let before = timeline.snapshot().clone();
    assert!(matches!(
        timeline.apply(envelope("stale", stale)),
        Err(TimelineError::StaleDistributionTarget(_))
    ));
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.events().len(), 1);
}
#[test]
fn group_authoring_is_stale_safe_and_undo_restores_default_group() {
    let (snapshot, _) = fixture();
    let expected = snapshot.entity(&"pool".into()).unwrap().clone();
    let groups = vec![PoolDistributionGroup {
        id: 1,
        name: "North".into(),
        pixels: vec![BlindPixel::new(2, 2), BlindPixel::new(3, 2)],
    }];
    let save = LevelCommand::SetPoolDistributionGroups {
        entity_id: "pool".into(),
        expected,
        groups,
    };
    let mut timeline = LevelTimeline::new(snapshot.clone()).unwrap();
    timeline.apply(envelope("groups", save.clone())).unwrap();
    let grouped = timeline.snapshot().clone();
    assert_eq!(
        grouped
            .entity(&"pool".into())
            .unwrap()
            .as_blind()
            .unwrap()
            .distribution_groups()
            .len(),
        1
    );
    assert!(matches!(
        timeline.apply(envelope("stale-groups", save)),
        Err(TimelineError::StaleDistributionTarget(_))
    ));
    assert_eq!(timeline.snapshot(), &grouped);
    timeline
        .undo_latest(CommandMetadata::new("undo", "alice", 2))
        .unwrap();
    assert_eq!(timeline.snapshot(), &snapshot);
}
#[test]
fn precommit_rejection_does_not_consume_distribution_command_or_mutate_history() {
    let (snapshot, request) = fixture();
    let mut timeline = LevelTimeline::new(snapshot.clone()).unwrap();
    let envelope = envelope("allocation", command(&snapshot, request));
    let rejected = timeline.apply_validated(envelope.clone(), |_, _| {
        Err(TimelineError::InvalidDistribution("budget rejected".into()))
    });
    assert!(rejected.is_err());
    assert_eq!(timeline.snapshot(), &snapshot);
    assert!(timeline.events().is_empty());
    assert!(matches!(
        timeline.apply(envelope).unwrap(),
        ApplyOutcome::Applied(_)
    ));
}
