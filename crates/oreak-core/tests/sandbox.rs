use oreak_core::{
    Block, CellKind, Decorator, DirectionMode, EntityId, GridPoint, GridSize, LevelError,
    LevelSnapshot, PlaceableEntity, SANDBOX_SAND_STEPPING_SUPPORTED, SandboxError,
    SandboxMoveDirection, SandboxSession, Shape, TimelineError,
};

const _: () = assert!(!SANDBOX_SAND_STEPPING_SUPPORTED);

fn block(id: &str, x: u16, y: u16) -> PlaceableEntity {
    PlaceableEntity::block(
        id,
        GridPoint::new(x, y),
        Shape::new(1, 1, 1).unwrap(),
        Block::default(),
    )
    .unwrap()
}

fn snapshot(
    entities: Vec<PlaceableEntity>,
    decorators: Vec<Decorator>,
    walls: &[GridPoint],
) -> LevelSnapshot {
    let size = GridSize::new(6, 6).unwrap();
    let mut cells = vec![CellKind::Floor; size.cell_count()];
    for wall in walls {
        cells[size.index_of(*wall).unwrap()] = CellKind::Wall;
    }
    LevelSnapshot::from_parts(size, cells, entities, decorators).unwrap()
}

#[test]
fn sandbox_moves_are_isolated_recorded_and_restartable() {
    let authored = snapshot(
        vec![block("mover", 1, 1), block("obstacle", 3, 1)],
        Vec::new(),
        &[],
    );
    let authored_hash = authored.content_hash();
    let mut session = SandboxSession::new(&authored).unwrap();

    session
        .move_entity(&EntityId::from("mover"), SandboxMoveDirection::Right)
        .unwrap();
    assert_eq!(
        session
            .snapshot()
            .entity(&EntityId::from("mover"))
            .unwrap()
            .origin(),
        GridPoint::new(2, 1)
    );
    assert_eq!(session.history().len(), 1);
    assert_eq!(authored.content_hash(), authored_hash);
    assert_eq!(
        session
            .authored_snapshot()
            .entity(&EntityId::from("mover"))
            .unwrap()
            .origin(),
        GridPoint::new(1, 1)
    );

    let error = session
        .move_entity(&EntityId::from("mover"), SandboxMoveDirection::Right)
        .unwrap_err();
    assert!(matches!(
        error,
        SandboxError::Timeline(TimelineError::InvalidLevel(
            LevelError::EntityOverlap { .. }
        ))
    ));
    assert_eq!(session.history().len(), 1);

    session.restart().unwrap();
    assert!(session.history().is_empty());
    assert_eq!(session.snapshot(), &authored);
}

#[test]
fn sandbox_respects_ice_and_direction_constraints() {
    let iced = snapshot(
        vec![block("mover", 1, 1)],
        vec![
            Decorator::ice_with_blocking_count("ice", "mover", 2),
            Decorator::direction_mode("direction", "mover", DirectionMode::Horizontal).unwrap(),
        ],
        &[],
    );
    let mut session = SandboxSession::new(&iced).unwrap();
    assert_eq!(
        session
            .move_entity(&EntityId::from("mover"), SandboxMoveDirection::Right)
            .unwrap_err(),
        SandboxError::BlockedByIce {
            entity_id: EntityId::from("mover"),
            blocking_count: 2,
        }
    );

    let directed = snapshot(
        vec![block("mover", 1, 1)],
        vec![
            Decorator::ice_with_blocking_count("ice", "mover", 0),
            Decorator::direction_mode("direction", "mover", DirectionMode::Horizontal).unwrap(),
        ],
        &[],
    );
    let mut session = SandboxSession::new(&directed).unwrap();
    assert_eq!(
        session
            .move_entity(&EntityId::from("mover"), SandboxMoveDirection::Up)
            .unwrap_err(),
        SandboxError::BlockedByDirection {
            entity_id: EntityId::from("mover"),
            mode: DirectionMode::Horizontal,
            attempted: SandboxMoveDirection::Up,
        }
    );
    session
        .move_entity(&EntityId::from("mover"), SandboxMoveDirection::Right)
        .unwrap();
    assert_eq!(session.history().len(), 1);
}

#[test]
fn sandbox_movement_respects_map_walls_and_explicitly_defers_sand() {
    let authored = snapshot(
        vec![block("edge", 0, 0), block("wall-mover", 2, 1)],
        Vec::new(),
        &[GridPoint::new(2, 2)],
    );
    let mut session = SandboxSession::new(&authored).unwrap();
    assert_eq!(
        session
            .move_entity(&EntityId::from("edge"), SandboxMoveDirection::Left)
            .unwrap_err(),
        SandboxError::MovementOutOfBounds {
            entity_id: EntityId::from("edge"),
            direction: SandboxMoveDirection::Left,
        }
    );
    assert!(matches!(
        session.move_entity(&EntityId::from("wall-mover"), SandboxMoveDirection::Up),
        Err(SandboxError::Timeline(TimelineError::InvalidLevel(
            LevelError::EntityOnWall { .. }
        )))
    ));
    assert_eq!(session.step_sand(), Err(SandboxError::SandSteppingDeferred));
    assert!(session.history().is_empty());
}
