use oreak_core::{
    ApplyOutcome, Blind, BlindGuide, BlindPixel, BlindTile, Block, CommandEnvelope,
    CommandMetadata, EntityError, EntityId, GridPoint, HistoryChange, LevelCommand,
    LevelCommandKind, LevelError, LevelSnapshot, LevelTarget, LevelTimeline, LevelValue,
    PlaceableEntity, Shape, TimelineError,
};

fn command(id: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, "artist", 100), command)
}

fn blind_entity(blind: Blind, shape: Shape) -> PlaceableEntity {
    PlaceableEntity::blind("blind", GridPoint::new(0, 0), shape, blind).unwrap()
}

#[test]
fn resamples_each_tile_by_nearest_source_pixel_center() {
    let shape = Shape::new(2, 1, 0b11).unwrap();
    let blind = Blind::new(
        2,
        vec![
            BlindTile::from_colors(2, vec![1, 2, 3, 4]).unwrap(),
            BlindTile::from_colors(2, vec![5, 6, 7, 8]).unwrap(),
        ],
    )
    .unwrap();

    let resampled = blind.resampled(shape, 3).unwrap();

    assert_eq!(resampled.pixels_per_cell(), 3);
    assert_eq!(resampled.tiles()[0].colors(), &[1, 2, 2, 3, 4, 4, 3, 4, 4]);
    assert_eq!(resampled.tiles()[1].colors(), &[5, 6, 6, 7, 8, 8, 7, 8, 8]);
}

#[test]
fn guide_projection_is_valid_bounded_and_drops_unrepresentable_topology() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let source_guide = BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap();
    let blind =
        Blind::with_guides(2, vec![BlindTile::empty(2).unwrap()], vec![source_guide]).unwrap();

    let upsampled = blind.resampled(shape, 4).unwrap();
    assert_eq!(
        upsampled.guides(),
        &[
            BlindGuide::new(BlindPixel::new(1, 0), BlindPixel::new(2, 0)).unwrap(),
            BlindGuide::new(BlindPixel::new(1, 1), BlindPixel::new(2, 1)).unwrap(),
        ]
    );

    // A one-pixel tile has no adjacency on which the source guide can exist.
    let downsampled = blind.resampled(shape, 1).unwrap();
    assert!(downsampled.guides().is_empty());
}

#[test]
fn resolution_command_records_entity_history_and_undo_restores_exactly() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let original = blind_entity(
        Blind::new(
            2,
            vec![BlindTile::from_colors(2, vec![1, 2, 3, 4]).unwrap()],
        )
        .unwrap(),
        shape,
    );
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 4).unwrap()).unwrap();
    timeline
        .apply(command(
            "place",
            LevelCommand::PlaceEntity {
                entity: original.clone(),
            },
        ))
        .unwrap();
    let original_hash = timeline.snapshot().content_hash();

    let ApplyOutcome::Applied(event) = timeline
        .apply(command(
            "resolution",
            LevelCommand::SetBlindResolution {
                entity_id: EntityId::from("blind"),
                pixels_per_cell: 3,
            },
        ))
        .unwrap()
    else {
        panic!("expected resolution change");
    };

    assert_eq!(event.command.kind(), LevelCommandKind::SetBlindResolution);
    assert!(matches!(
        event.inverse,
        LevelCommand::RestoreEntity { ref entity } if entity == &original
    ));
    assert_eq!(
        event.changes,
        vec![HistoryChange {
            target: LevelTarget::Entity(EntityId::from("blind")),
            before: LevelValue::Entity(Some(original.clone())),
            after: LevelValue::Entity(
                timeline
                    .snapshot()
                    .entity(&EntityId::from("blind"))
                    .cloned()
            ),
        }]
    );
    assert_eq!(
        timeline
            .blame_entity(&EntityId::from("blind"))
            .unwrap()
            .command_kind,
        LevelCommandKind::SetBlindResolution
    );
    assert_eq!(
        timeline.history_for_entity(&EntityId::from("blind")).len(),
        2
    );

    timeline
        .undo_latest(CommandMetadata::new("undo", "artist", 200))
        .unwrap();
    assert_eq!(
        timeline.snapshot().entity(&EntityId::from("blind")),
        Some(&original)
    );
    assert_eq!(timeline.snapshot().content_hash(), original_hash);
}

#[test]
fn resolution_validation_and_no_op_do_not_consume_command_ids() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let blind = Blind::new(2, vec![BlindTile::empty(2).unwrap()]).unwrap();
    assert_eq!(
        blind.resampled(shape, 0),
        Err(EntityError::InvalidPixelsPerCell(0))
    );
    assert_eq!(
        blind.resampled(shape, 33),
        Err(EntityError::InvalidPixelsPerCell(33))
    );

    let mut timeline = LevelTimeline::new(LevelSnapshot::new(4, 4).unwrap()).unwrap();
    timeline
        .apply(command(
            "place",
            LevelCommand::PlaceEntity {
                entity: blind_entity(blind, shape),
            },
        ))
        .unwrap();
    let event_count = timeline.events().len();
    assert!(matches!(
        timeline
            .apply(command(
                "reusable",
                LevelCommand::SetBlindResolution {
                    entity_id: EntityId::from("blind"),
                    pixels_per_cell: 2,
                },
            ))
            .unwrap(),
        ApplyOutcome::NoChange { .. }
    ));
    assert_eq!(timeline.events().len(), event_count);

    timeline
        .apply(command(
            "reusable",
            LevelCommand::SetBlindResolution {
                entity_id: EntityId::from("blind"),
                pixels_per_cell: 1,
            },
        ))
        .unwrap();
    assert_eq!(timeline.events().len(), event_count + 1);

    assert!(matches!(
        timeline.apply(command(
            "invalid",
            LevelCommand::SetBlindResolution {
                entity_id: EntityId::from("blind"),
                pixels_per_cell: 0,
            },
        )),
        Err(TimelineError::InvalidLevel(LevelError::InvalidEntity(
            EntityError::InvalidPixelsPerCell(0)
        )))
    ));

    let block =
        PlaceableEntity::block("block", GridPoint::new(2, 2), shape, Block::default()).unwrap();
    assert_eq!(
        block.resampled_blind(2),
        Err(EntityError::NotBlind(EntityId::from("block")))
    );
}
