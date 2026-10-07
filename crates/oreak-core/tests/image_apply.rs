use oreak_core::{
    ApplyOutcome, Blind, BlindGuide, BlindGuideSet, BlindPixel, BlindStroke, BlindTile, Block,
    CellKind, CommandEnvelope, CommandMetadata, EntityId, GridPoint, ImagePlacement, ImageSampling,
    ImageTransparency, IndexedImage, LevelCommand, LevelCommandKind, LevelSnapshot, LevelTarget,
    LevelTimeline, PALETTE_VERSION, PaletteSettings, PlaceableEntity, Shape, TimelineError,
};

fn command(id: &str, actor: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, actor, 100), command)
}

fn pool(id: &str, x: u16) -> PlaceableEntity {
    PlaceableEntity::blind(
        id,
        GridPoint::new(x, 0),
        Shape::new(1, 1, 1).unwrap(),
        Blind::new(2, vec![BlindTile::empty(2).unwrap()]).unwrap(),
    )
    .unwrap()
}

fn seeded(targets: &[PlaceableEntity]) -> LevelTimeline {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(64, 64).unwrap()).unwrap();
    timeline
        .apply(command(
            "seed",
            "seed",
            LevelCommand::PlaceEntities {
                entities: targets.to_vec(),
            },
        ))
        .unwrap();
    timeline
}

fn image_command(targets: Vec<PlaceableEntity>) -> LevelCommand {
    LevelCommand::ApplyImageToPools {
        image: IndexedImage {
            width: 1,
            height: 1,
            pixels: vec![3],
            palette_version: PALETTE_VERSION,
        },
        settings: PaletteSettings::default(),
        placement: ImagePlacement {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
            sampling: ImageSampling::Nearest,
            pixelation: 1,
            resolution: None,
            transparency: ImageTransparency::Preserve,
        },
        targets,
    }
}

#[test]
fn applies_multiple_pools_as_one_event_with_one_exact_undo() {
    let targets = vec![pool("a", 0), pool("b", 1)];
    let mut timeline = seeded(&targets);
    let before = timeline.snapshot().clone();
    let ApplyOutcome::Applied(event) = timeline
        .apply(command("image", "artist", image_command(targets.clone())))
        .unwrap()
    else {
        panic!("expected image event")
    };
    assert_eq!(event.command.kind(), LevelCommandKind::ApplyImageToPools);
    assert_eq!(event.sequence, 2);
    assert_eq!(event.changes.len(), 2);
    assert_eq!(event.before_hash, before.content_hash());
    assert_ne!(event.before_hash, event.after_hash);
    assert!(
        matches!(event.inverse, LevelCommand::RestoreEntities { ref entities } if entities == &targets)
    );
    for entity in &targets {
        assert!(
            event
                .changes
                .iter()
                .any(|change| change.target == LevelTarget::Entity(entity.id().clone()))
        );
        let current = timeline.snapshot().entity(entity.id()).unwrap();
        assert_eq!(current.origin(), entity.origin());
        assert_eq!(current.shape(), entity.shape());
        assert_eq!(current.as_blind().unwrap().tiles()[0].colors(), &[3; 4]);
    }
    let undo = timeline
        .undo_latest(CommandMetadata::new("undo", "artist", 200))
        .unwrap();
    assert_eq!(undo.reverts_sequence, Some(event.sequence));
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(undo.after_hash, before.content_hash());
}

#[test]
fn stale_second_target_rejects_entire_apply_and_does_not_consume_id() {
    let targets = vec![pool("a", 0), pool("b", 1)];
    let mut timeline = seeded(&targets);
    timeline
        .apply(command(
            "move",
            "collaborator",
            LevelCommand::MoveEntity {
                entity_id: EntityId::from("b"),
                origin: GridPoint::new(3, 0),
            },
        ))
        .unwrap();
    let before = timeline.snapshot().clone();
    let events = timeline.events().len();
    assert_eq!(
        timeline.apply(command("image", "artist", image_command(targets))),
        Err(TimelineError::StaleImageTarget(EntityId::from("b")))
    );
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.events().len(), events);
    let fresh = vec![
        before.entity(&EntityId::from("a")).unwrap().clone(),
        before.entity(&EntityId::from("b")).unwrap().clone(),
    ];
    timeline
        .apply(command("image", "artist", image_command(fresh)))
        .unwrap();
    assert_eq!(timeline.events().len(), events + 1);
}

#[test]
fn expected_snapshot_guard_detects_paint_resolution_guides_and_deletion() {
    let original = pool("a", 0);
    let operations = vec![
        LevelCommand::PaintBlindStroke {
            entity_id: EntityId::from("a"),
            color_index: 4,
            stroke: BlindStroke::new(vec![BlindPixel::new(0, 0)]).unwrap(),
        },
        LevelCommand::SetBlindResolution {
            entity_id: EntityId::from("a"),
            pixels_per_cell: 3,
        },
        LevelCommand::AddBlindGuides {
            entity_id: EntityId::from("a"),
            guides: BlindGuideSet::new(vec![
                BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap(),
            ])
            .unwrap(),
        },
        LevelCommand::DeleteEntity {
            entity_id: EntityId::from("a"),
        },
    ];
    for operation in operations {
        let mut timeline = seeded(std::slice::from_ref(&original));
        timeline
            .apply(command("other", "collaborator", operation))
            .unwrap();
        let before = timeline.snapshot().clone();
        assert_eq!(
            timeline.apply(command(
                "image",
                "artist",
                image_command(vec![original.clone()])
            )),
            Err(TimelineError::StaleImageTarget(EntityId::from("a")))
        );
        assert_eq!(timeline.snapshot(), &before);
        assert_eq!(timeline.events().len(), 2);
    }
}

#[test]
fn unrelated_edits_do_not_stale_targets_or_block_image_undo() {
    let targets = vec![pool("a", 0), pool("b", 1)];
    let mut timeline = seeded(&targets);
    timeline
        .apply(command(
            "other",
            "collaborator",
            LevelCommand::SetCell {
                point: GridPoint::new(10, 10),
                kind: CellKind::Wall,
            },
        ))
        .unwrap();
    timeline
        .apply(command("image", "artist", image_command(targets.clone())))
        .unwrap();
    timeline
        .apply(command(
            "later",
            "collaborator",
            LevelCommand::SetCell {
                point: GridPoint::new(11, 10),
                kind: CellKind::Wall,
            },
        ))
        .unwrap();
    timeline
        .undo_latest(CommandMetadata::new("undo", "artist", 200))
        .unwrap();
    for target in &targets {
        assert_eq!(timeline.snapshot().entity(target.id()), Some(target));
    }
    assert_eq!(
        timeline.snapshot().cell(GridPoint::new(10, 10)).unwrap(),
        CellKind::Wall
    );
    assert_eq!(
        timeline.snapshot().cell(GridPoint::new(11, 10)).unwrap(),
        CellKind::Wall
    );
}

#[test]
fn later_target_paint_blocks_whole_image_undo_without_partial_restore() {
    let targets = vec![pool("a", 0), pool("b", 1)];
    let mut timeline = seeded(&targets);
    timeline
        .apply(command("image", "artist", image_command(targets)))
        .unwrap();
    timeline
        .apply(command(
            "paint",
            "collaborator",
            LevelCommand::PaintBlindStroke {
                entity_id: EntityId::from("b"),
                color_index: 4,
                stroke: BlindStroke::new(vec![BlindPixel::new(0, 0)]).unwrap(),
            },
        ))
        .unwrap();
    let before = timeline.snapshot().clone();
    assert!(matches!(
        timeline.undo_latest(CommandMetadata::new("undo", "artist", 200)),
        Err(TimelineError::UndoConflict { .. })
    ));
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.events().len(), 3);
}

#[test]
fn rejects_empty_duplicate_non_pool_and_invalid_projection_atomically() {
    let original = pool("a", 0);
    let block = PlaceableEntity::block(
        "block",
        GridPoint::new(2, 0),
        Shape::new(1, 1, 1).unwrap(),
        Block::default(),
    )
    .unwrap();
    let mut timeline = seeded(&[original.clone(), block.clone()]);
    let before = timeline.snapshot().clone();
    let mut invalid_projection = image_command(vec![original.clone()]);
    if let LevelCommand::ApplyImageToPools { placement, .. } = &mut invalid_projection {
        placement.width = f64::NAN;
    }
    for invalid in [
        image_command(vec![]),
        image_command(vec![original.clone(), original]),
        image_command(vec![block]),
        invalid_projection,
    ] {
        assert!(matches!(
            timeline.apply(command("reusable", "artist", invalid)),
            Err(TimelineError::InvalidImageApply(_))
        ));
        assert_eq!(timeline.snapshot(), &before);
        assert_eq!(timeline.events().len(), 1);
    }
}

#[test]
fn enforces_pool_count_and_aggregate_source_and_output_pixel_budgets() {
    let mut excessive_count = vec![];
    for n in 0..65 {
        excessive_count.push(pool(&format!("p{n}"), 0));
    }
    let mut empty = LevelTimeline::new(LevelSnapshot::new(64, 64).unwrap()).unwrap();
    assert!(matches!(
        empty.apply(command("large", "artist", image_command(excessive_count))),
        Err(TimelineError::InvalidImageApply(_))
    ));
    assert!(empty.events().is_empty());

    for (source_resolution, output_resolution) in [(1, 32), (32, 1)] {
        let shape = Shape::new(8, 8, u64::MAX).unwrap();
        let targets: Vec<_> = (0..16)
            .map(|n| {
                PlaceableEntity::blind(
                    format!("p{n}"),
                    GridPoint::new((n % 8) * 8, (n / 8) * 8),
                    shape,
                    Blind::new(
                        source_resolution,
                        vec![BlindTile::empty(source_resolution).unwrap(); 64],
                    )
                    .unwrap(),
                )
                .unwrap()
            })
            .collect();
        let mut timeline = seeded(&targets);
        let before_hash = timeline.snapshot().content_hash();
        let mut apply = image_command(targets);
        if let LevelCommand::ApplyImageToPools { placement, .. } = &mut apply {
            placement.resolution = Some(output_resolution);
        }
        assert!(matches!(
            timeline.apply(command("large", "artist", apply)),
            Err(TimelineError::InvalidImageApply(_))
        ));
        assert_eq!(timeline.snapshot().content_hash(), before_hash);
        assert_eq!(timeline.events().len(), 1);
    }
}

#[test]
fn exact_reapply_is_no_change_without_consuming_command_id() {
    let mut timeline = seeded(&[pool("a", 0), pool("b", 1)]);
    let targets = timeline.snapshot().entities().to_vec();
    timeline
        .apply(command("image", "artist", image_command(targets)))
        .unwrap();
    let targets = timeline.snapshot().entities().to_vec();
    let before_hash = timeline.snapshot().content_hash();
    assert_eq!(
        timeline
            .apply(command(
                "reusable",
                "artist",
                image_command(targets.clone())
            ))
            .unwrap(),
        ApplyOutcome::NoChange {
            snapshot_hash: before_hash
        }
    );
    assert_eq!(timeline.events().len(), 2);
    let mut changed = image_command(targets);
    if let LevelCommand::ApplyImageToPools { image, .. } = &mut changed {
        image.pixels[0] = 4;
    }
    timeline
        .apply(command("reusable", "artist", changed))
        .unwrap();
    assert_eq!(timeline.events().len(), 3);
}

#[test]
fn image_resolution_change_undo_restores_guides_and_original_pixels() {
    let mut timeline = seeded(&[pool("a", 0), pool("b", 1)]);
    timeline
        .apply(command(
            "guide",
            "seed",
            LevelCommand::AddBlindGuides {
                entity_id: EntityId::from("a"),
                guides: BlindGuideSet::new(vec![
                    BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap(),
                ])
                .unwrap(),
            },
        ))
        .unwrap();
    let before = timeline.snapshot().clone();
    let mut apply = image_command(before.entities().to_vec());
    if let LevelCommand::ApplyImageToPools { placement, .. } = &mut apply {
        placement.resolution = Some(3);
    }
    timeline.apply(command("image", "artist", apply)).unwrap();
    for entity in timeline.snapshot().entities() {
        assert_eq!(entity.as_blind().unwrap().pixels_per_cell(), 3);
    }
    timeline
        .undo_latest(CommandMetadata::new("undo", "artist", 200))
        .unwrap();
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.snapshot().content_hash(), before.content_hash());
}

#[test]
fn apply_validator_rejection_preserves_history_provenance_sequence_and_id() {
    let targets = vec![pool("a", 0), pool("b", 1)];
    let mut timeline = seeded(&targets);
    let before = timeline.snapshot().clone();
    let events = timeline.events().to_vec();
    let blame = timeline.blame_entity(&EntityId::from("a"));
    let envelope = command("image", "artist", image_command(targets));
    let rejected = TimelineError::InvalidImageApply("transport budget".to_owned());
    let mut called = false;
    assert_eq!(
        timeline.apply_validated(envelope.clone(), |event, snapshot| {
            called = true;
            assert_eq!(event.sequence, 2);
            assert_eq!(event.before_hash, before.content_hash());
            assert_eq!(event.after_hash, snapshot.content_hash());
            assert_ne!(snapshot, &before);
            Err(rejected.clone())
        }),
        Err(rejected)
    );
    assert!(called);
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.events(), events.as_slice());
    assert_eq!(timeline.blame_entity(&EntityId::from("a")), blame);
    let ApplyOutcome::Applied(event) = timeline.apply(envelope).unwrap() else {
        panic!("expected event")
    };
    assert_eq!(event.sequence, 2);
    assert_eq!(
        timeline
            .blame_entity(&EntityId::from("a"))
            .unwrap()
            .sequence,
        2
    );
}

#[test]
fn undo_validator_rejection_keeps_authored_event_active_and_id_reusable() {
    let targets = vec![pool("a", 0), pool("b", 1)];
    let mut timeline = seeded(&targets);
    let original = timeline.snapshot().clone();
    timeline
        .apply(command("image", "artist", image_command(targets)))
        .unwrap();
    let before = timeline.snapshot().clone();
    let events = timeline.events().to_vec();
    let blame = timeline.blame_entity(&EntityId::from("a"));
    let metadata = CommandMetadata::new("undo", "artist", 200);
    let error = TimelineError::InvalidImageApply("transport budget".to_owned());
    assert_eq!(
        timeline.undo_latest_validated(metadata.clone(), |event, snapshot| {
            assert_eq!(event.sequence, 3);
            assert_eq!(event.reverts_sequence, Some(2));
            assert_eq!(snapshot, &original);
            Err(error.clone())
        }),
        Err(error)
    );
    assert_eq!(timeline.snapshot(), &before);
    assert_eq!(timeline.events(), events.as_slice());
    assert_eq!(timeline.blame_entity(&EntityId::from("a")), blame);
    let undo = timeline.undo_latest(metadata).unwrap();
    assert_eq!(undo.sequence, 3);
    assert_eq!(undo.reverts_sequence, Some(2));
    assert_eq!(timeline.snapshot(), &original);
}

#[test]
fn command_roundtrip_and_replay_preserve_hashes() {
    let targets = vec![pool("a", 0), pool("b", 1)];
    let command_value = image_command(targets.clone());
    let json = serde_json::to_string(&command_value).unwrap();
    let decoded: LevelCommand = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, command_value);
    let mut first = seeded(&targets);
    let mut second = seeded(&targets);
    first
        .apply(command("image", "artist", command_value))
        .unwrap();
    second.apply(command("image", "artist", decoded)).unwrap();
    assert_eq!(first.events(), second.events());
    assert_eq!(
        first.snapshot().content_hash(),
        second.snapshot().content_hash()
    );
}
