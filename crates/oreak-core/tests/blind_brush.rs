use oreak_core::{
    ApplyOutcome, Blind, BlindGuide, BlindGuideSet, BlindPixel, BlindStroke, BlindTile, BrushError,
    CommandEnvelope, CommandMetadata, EntityError, EntityId, GridPoint, LevelCommand, LevelError,
    LevelSnapshot, LevelTimeline, PlaceableEntity, Shape, ShapeCell, TimelineError,
};

fn command(id: &str, command: LevelCommand) -> CommandEnvelope {
    CommandEnvelope::new(CommandMetadata::new(id, "artist", 100), command)
}

fn blind_entity(id: &str, shape: Shape, pixels_per_cell: u8) -> PlaceableEntity {
    let tiles = (0..shape.occupied_count())
        .map(|_| BlindTile::empty(pixels_per_cell).unwrap())
        .collect();
    PlaceableEntity::blind(
        id,
        GridPoint::new(0, 0),
        shape,
        Blind::new(pixels_per_cell, tiles).unwrap(),
    )
    .unwrap()
}

fn timeline(entity: PlaceableEntity) -> LevelTimeline {
    let mut timeline = LevelTimeline::new(LevelSnapshot::new(8, 8).unwrap()).unwrap();
    timeline
        .apply(command("place", LevelCommand::PlaceEntity { entity }))
        .unwrap();
    timeline
}

fn color(timeline: &LevelTimeline, entity_id: &str, pixel: BlindPixel) -> Option<u8> {
    let entity = timeline
        .snapshot()
        .entity(&EntityId::from(entity_id))
        .unwrap();
    entity.as_blind().unwrap().color_at(entity.shape(), pixel)
}

#[test]
fn strokes_clip_to_bounds_and_shape_holes_and_validate_colors() {
    let ring = Shape::new(3, 3, 0b111_101_111).unwrap();
    let mut timeline = timeline(blind_entity("blind", ring, 2));
    let stroke = BlindStroke::new(vec![
        BlindPixel::new(0, 0),
        BlindPixel::new(2, 2),
        BlindPixel::new(3, 3),
        BlindPixel::new(4, 4),
        BlindPixel::new(100, 100),
        BlindPixel::new(0, 0),
    ])
    .unwrap();
    assert_eq!(stroke.pixels().len(), 5);

    let ApplyOutcome::Applied(event) = timeline
        .apply(command(
            "paint",
            LevelCommand::PaintBlindStroke {
                entity_id: EntityId::from("blind"),
                color_index: 10,
                stroke,
            },
        ))
        .unwrap()
    else {
        panic!("expected clipped paint");
    };
    assert_eq!(event.changes.len(), 2);
    assert_eq!(color(&timeline, "blind", BlindPixel::new(0, 0)), Some(10));
    assert_eq!(color(&timeline, "blind", BlindPixel::new(3, 3)), None);
    assert!(
        timeline
            .blame_blind_tile(&EntityId::from("blind"), ShapeCell::new(0, 0))
            .is_some()
    );

    let events = timeline.events().len();
    let outside = BlindStroke::new(vec![BlindPixel::new(100, 100)]).unwrap();
    let outcome = timeline
        .apply(command(
            "reusable",
            LevelCommand::EraseBlindStroke {
                entity_id: EntityId::from("blind"),
                stroke: outside,
            },
        ))
        .unwrap();
    assert!(matches!(outcome, ApplyOutcome::NoChange { .. }));
    assert_eq!(timeline.events().len(), events);

    timeline
        .apply(command(
            "reusable",
            LevelCommand::EraseBlindStroke {
                entity_id: EntityId::from("blind"),
                stroke: BlindStroke::new(vec![BlindPixel::new(0, 0)]).unwrap(),
            },
        ))
        .unwrap();
    assert_eq!(color(&timeline, "blind", BlindPixel::new(0, 0)), Some(0));
    timeline
        .undo_latest(CommandMetadata::new("undo-erase", "artist", 200))
        .unwrap();
    assert_eq!(color(&timeline, "blind", BlindPixel::new(0, 0)), Some(10));

    let before = timeline.events().len();
    assert!(matches!(
        timeline.apply(command(
            "invalid-color",
            LevelCommand::PaintBlindStroke {
                entity_id: EntityId::from("blind"),
                color_index: 0,
                stroke: BlindStroke::new(vec![BlindPixel::new(0, 0)]).unwrap(),
            }
        )),
        Err(TimelineError::InvalidLevel(LevelError::InvalidEntity(
            EntityError::InvalidBrush(BrushError::InvalidColorIndex(0))
        )))
    ));
    assert_eq!(timeline.events().len(), before);
}

#[test]
fn flood_fill_is_four_neighbor_and_respects_guide_barriers() {
    let mut timeline = timeline(blind_entity("blind", Shape::new(1, 1, 1).unwrap(), 2));
    let bottom = BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap();
    let top = BlindGuide::new(BlindPixel::new(0, 1), BlindPixel::new(1, 1)).unwrap();
    let guides = BlindGuideSet::new(vec![top, bottom]).unwrap();
    let ApplyOutcome::Applied(event) = timeline
        .apply(command(
            "guides",
            LevelCommand::AddBlindGuides {
                entity_id: EntityId::from("blind"),
                guides: guides.clone(),
            },
        ))
        .unwrap()
    else {
        panic!("expected guides");
    };
    assert_eq!(event.changes.len(), 2);
    assert!(
        timeline
            .blame_blind_guide(&EntityId::from("blind"), bottom)
            .is_some()
    );
    let entity = timeline
        .snapshot()
        .entity(&EntityId::from("blind"))
        .unwrap();
    let partition = entity
        .as_blind()
        .unwrap()
        .paintable_partition(entity.shape(), BlindPixel::new(0, 0));
    assert_eq!(partition.len(), 2);
    assert!(partition.contains(&BlindPixel::new(0, 1)));
    assert!(!partition.contains(&BlindPixel::new(1, 0)));

    timeline
        .apply(command(
            "fill-left",
            LevelCommand::FloodFillBlind {
                entity_id: EntityId::from("blind"),
                start: BlindPixel::new(0, 0),
                color_index: 4,
            },
        ))
        .unwrap();
    assert_eq!(color(&timeline, "blind", BlindPixel::new(0, 0)), Some(4));
    assert_eq!(color(&timeline, "blind", BlindPixel::new(0, 1)), Some(4));
    assert_eq!(color(&timeline, "blind", BlindPixel::new(1, 0)), Some(0));
    assert_eq!(color(&timeline, "blind", BlindPixel::new(1, 1)), Some(0));

    let event_count = timeline.events().len();
    let painted_fill = timeline
        .apply(command(
            "fill-painted",
            LevelCommand::FloodFillBlind {
                entity_id: EntityId::from("blind"),
                start: BlindPixel::new(0, 0),
                color_index: 7,
            },
        ))
        .unwrap();
    assert!(matches!(painted_fill, ApplyOutcome::NoChange { .. }));
    assert_eq!(timeline.events().len(), event_count);
    assert_eq!(color(&timeline, "blind", BlindPixel::new(0, 0)), Some(4));

    let event_count = timeline.events().len();
    let outcome = timeline
        .apply(command(
            "fill-noop",
            LevelCommand::FloodFillBlind {
                entity_id: EntityId::from("blind"),
                start: BlindPixel::new(0, 0),
                color_index: 4,
            },
        ))
        .unwrap();
    assert!(matches!(outcome, ApplyOutcome::NoChange { .. }));
    assert_eq!(timeline.events().len(), event_count);

    timeline
        .undo_latest(CommandMetadata::new("undo-fill", "artist", 200))
        .unwrap();
    for y in 0..2 {
        for x in 0..2 {
            assert_eq!(color(&timeline, "blind", BlindPixel::new(x, y)), Some(0));
        }
    }
}

#[test]
fn guide_add_remove_no_ops_and_undo_are_deterministic() {
    let mut timeline = timeline(blind_entity("blind", Shape::new(1, 1, 1).unwrap(), 2));
    let guide = BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap();
    let set = BlindGuideSet::new(vec![guide, guide]).unwrap();
    assert_eq!(set.guides().len(), 1);
    timeline
        .apply(command(
            "add",
            LevelCommand::AddBlindGuides {
                entity_id: EntityId::from("blind"),
                guides: set.clone(),
            },
        ))
        .unwrap();

    let event_count = timeline.events().len();
    assert!(matches!(
        timeline
            .apply(command(
                "add-again",
                LevelCommand::AddBlindGuides {
                    entity_id: EntityId::from("blind"),
                    guides: set.clone(),
                },
            ))
            .unwrap(),
        ApplyOutcome::NoChange { .. }
    ));
    assert_eq!(timeline.events().len(), event_count);

    timeline
        .apply(command(
            "remove",
            LevelCommand::RemoveBlindGuides {
                entity_id: EntityId::from("blind"),
                guides: set,
            },
        ))
        .unwrap();
    assert!(
        !timeline
            .snapshot()
            .entity(&EntityId::from("blind"))
            .unwrap()
            .as_blind()
            .unwrap()
            .has_guide(guide)
    );
    timeline
        .undo_latest(CommandMetadata::new("undo-remove", "artist", 200))
        .unwrap();
    assert!(
        timeline
            .snapshot()
            .entity(&EntityId::from("blind"))
            .unwrap()
            .as_blind()
            .unwrap()
            .has_guide(guide)
    );
}

#[test]
fn brush_input_and_guide_serde_are_bounded_and_validated() {
    assert!(matches!(
        BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(2, 0)),
        Err(BrushError::GuideEndpointsNotAdjacent { .. })
    ));
    let error =
        serde_json::from_str::<BlindGuide>(r#"{"first":{"x":0,"y":0},"second":{"x":1,"y":1}}"#)
            .unwrap_err();
    assert!(error.to_string().contains("not four-neighbor adjacent"));

    let entity = blind_entity("blind", Shape::new(1, 1, 1).unwrap(), 1);
    let mut json = serde_json::to_value(entity).unwrap();
    json["kind"]["Blind"]["tiles"][0]["colors"] = serde_json::json!([11]);
    assert!(
        serde_json::from_value::<PlaceableEntity>(json)
            .unwrap_err()
            .to_string()
            .contains("0..=10")
    );
}

#[test]
fn tile_provenance_allows_conflict_safe_independent_undo() {
    let mut timeline = timeline(blind_entity("blind", Shape::new(2, 1, 0b11).unwrap(), 1));
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("alice-paint", "alice", 100),
            LevelCommand::PaintBlindStroke {
                entity_id: EntityId::from("blind"),
                color_index: 2,
                stroke: BlindStroke::new(vec![BlindPixel::new(0, 0)]).unwrap(),
            },
        ))
        .unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("bob-paint", "bob", 101),
            LevelCommand::PaintBlindStroke {
                entity_id: EntityId::from("blind"),
                color_index: 3,
                stroke: BlindStroke::new(vec![BlindPixel::new(1, 0)]).unwrap(),
            },
        ))
        .unwrap();

    timeline
        .undo_latest(CommandMetadata::new("alice-undo", "alice", 200))
        .unwrap();
    assert_eq!(color(&timeline, "blind", BlindPixel::new(0, 0)), Some(0));
    assert_eq!(color(&timeline, "blind", BlindPixel::new(1, 0)), Some(3));
}
