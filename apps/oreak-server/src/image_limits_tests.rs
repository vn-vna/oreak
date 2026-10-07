use super::*;
use oreak_core::{
    Blind, BlindTile, CellKind, CommandMetadata, GridSize, ImagePlacement, ImageSampling,
    ImageTransparency, IndexedImage, PALETTE_VERSION, PaletteSettings, PlaceableEntity, Shape,
};

fn pool(id: &str, x: u16, y: u16, shape: Shape, resolution: u8) -> PlaceableEntity {
    PlaceableEntity::blind(
        id,
        GridPoint::new(x, y),
        shape,
        Blind::new(
            resolution,
            vec![
                BlindTile::from_colors(resolution, vec![1; usize::from(resolution).pow(2)])
                    .unwrap();
                shape.occupied_cells().count()
            ],
        )
        .unwrap(),
    )
    .unwrap()
}

fn image_command(id: &str, targets: Vec<PlaceableEntity>, color: u8) -> CommandEnvelope {
    CommandEnvelope::new(
        CommandMetadata::new(id, "artist", 1),
        LevelCommand::ApplyImageToPools {
            image: IndexedImage {
                width: 1024,
                height: 1024,
                pixels: vec![color; 1024 * 1024],
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
        },
    )
}

async fn service_with_pools(pools: Vec<PlaceableEntity>) -> (OreakRpcService, ProjectLevelTarget) {
    let service = OreakRpcService::new(mvp::MvpState::new(false));
    let target = ProjectLevelTarget::new("project", "level");
    let level = service.level(&target).await;
    let snapshot = LevelSnapshot::from_parts(
        GridSize::new(64, 64).unwrap(),
        vec![CellKind::Floor; 64 * 64],
        pools,
        vec![],
    )
    .unwrap();
    *level.timeline.lock().await = LevelTimeline::new(snapshot).unwrap();
    (service, target)
}

#[tokio::test]
async fn image_limit_rejects_oversized_apply_before_commit_or_broadcast() {
    let shape = Shape::new(8, 8, u64::MAX).unwrap();
    let pools: Vec<_> = (0..15)
        .map(|n| pool(&format!("pool-{n}"), (n % 8) * 8, (n / 8) * 8, shape, 32))
        .collect();
    let (service, target) = service_with_pools(pools.clone()).await;
    let level = service.level(&target).await;
    let before = level.timeline.lock().await.snapshot().content_hash();
    let mut receiver = level.events.subscribe();
    let error = service
        .commit_command(target.clone(), image_command("retry", pools, 2))
        .await
        .unwrap_err();
    assert_eq!(error.code(), RpcErrorCode::InvalidCommand.json_rpc_code());
    let timeline = level.timeline.lock().await;
    assert_eq!(timeline.snapshot().content_hash(), before);
    assert!(timeline.events().is_empty());
    drop(timeline);
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    // Rejection does not consume the ID, and a maximum-sized image still fits
    // when only one small expected Pool is projected.
    let one = level.timeline.lock().await.snapshot().entities()[0].clone();
    let result = service
        .commit_command(target, image_command("retry", vec![one], 2))
        .await
        .unwrap();
    assert_eq!(result.server_sequence, 1);
}

#[tokio::test]
async fn image_limit_history_pages_by_bytes_without_skips_or_empty_pages() {
    let (service, target) =
        service_with_pools(vec![pool("pool", 0, 0, Shape::new(1, 1, 1).unwrap(), 1)]).await;
    let level = service.level(&target).await;
    for n in 0..5 {
        let pools = level.timeline.lock().await.snapshot().entities().to_vec();
        let response = service
            .commit_command(
                target.clone(),
                image_command(&format!("image-{n}"), pools, 2 + (n % 2)),
            )
            .await
            .unwrap();
        ensure_rpc_payload_fits(&response).unwrap();
    }
    let timeline = level.timeline.lock().await;
    let mut before_sequence = None;
    let mut sequences = Vec::new();
    let mut pages = 0;
    loop {
        let page = bounded_history_response(
            &timeline,
            LevelHistoryRequest {
                target: target.clone(),
                before_sequence,
                limit: MAX_LEVEL_HISTORY_PAGE_SIZE,
            },
        )
        .unwrap();
        ensure_rpc_payload_fits(&page).unwrap();
        assert!(!page.events.is_empty());
        pages += 1;
        sequences.extend(page.events.iter().map(|event| event.sequence));
        if !page.has_more {
            assert_eq!(page.next_before_sequence, None);
            break;
        }
        assert_eq!(
            page.next_before_sequence,
            page.events.last().map(|event| event.sequence)
        );
        before_sequence = page.next_before_sequence;
    }
    assert!(
        pages > 1,
        "the byte cap must split otherwise-valid 1M-pixel image events"
    );
    assert_eq!(sequences, vec![5, 4, 3, 2, 1]);
    drop(timeline);
    let undo = service
        .commit_undo(target, CommandMetadata::new("undo", "artist", 2))
        .await
        .unwrap();
    assert_eq!(undo.event.reverts_sequence, Some(5));
    ensure_rpc_payload_fits(&undo).unwrap();
}

#[tokio::test]
async fn image_limit_oversized_undo_keeps_active_event_and_state() {
    let shape = Shape::new(8, 8, u64::MAX).unwrap();
    let pools: Vec<_> = (0..15)
        .map(|n| pool(&format!("pool-{n}"), (n % 8) * 8, (n / 8) * 8, shape, 32))
        .collect();
    let (service, target) = service_with_pools(pools.clone()).await;
    let level = service.level(&target).await;
    // Simulate pre-limit history: core remains transport-independent.
    level
        .timeline
        .lock()
        .await
        .apply(image_command("old-image", pools, 2))
        .unwrap();
    let before = level.timeline.lock().await.snapshot().content_hash();
    let mut receiver = level.events.subscribe();
    let result = service
        .commit_undo(target, CommandMetadata::new("undo", "artist", 2))
        .await;
    assert!(result.is_err());
    let timeline = level.timeline.lock().await;
    assert_eq!(timeline.snapshot().content_hash(), before);
    assert_eq!(timeline.events().len(), 1);
    assert!(actor_has_active_image_command(
        &timeline,
        &ActorId::from("artist")
    ));
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}
