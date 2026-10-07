use oreak_core::{
    Blind, BlindPixel, BlindTile, GridPoint, PlaceableEntity, PoolBoundary, PoolBoundaryError,
    Shape, pool_playable_mask,
};

fn pool(
    shape: Shape,
    resolution: u8,
    padding: Option<u16>,
    radius: Option<u16>,
) -> PlaceableEntity {
    let tile = BlindTile::from_colors(resolution, vec![1; usize::from(resolution).pow(2)]).unwrap();
    let blind = Blind::new(resolution, vec![tile; shape.occupied_count() as usize])
        .unwrap()
        .with_boundary(PoolBoundary {
            padding_pixels: padding,
            corner_radius_pixels: radius,
        })
        .unwrap();
    PlaceableEntity::blind("pool", GridPoint::new(0, 0), shape, blind).unwrap()
}
fn count(entity: &PlaceableEntity) -> usize {
    let mask = pool_playable_mask(entity).unwrap();
    (0..mask.height)
        .flat_map(|y| (0..mask.width).map(move |x| BlindPixel::new(x, y)))
        .filter(|pixel| mask.contains(*pixel))
        .count()
}
#[test]
fn matches_unity_eight_pixel_padding_and_corner_fixture() {
    let entity = pool(Shape::new(1, 1, 1).unwrap(), 8, Some(1), Some(2));
    let before = entity.clone();
    assert_eq!(count(&entity), 32); // 64 painted - 28 padding - 4 rounded corners.
    assert_eq!(entity, before);
    let mask = pool_playable_mask(&entity).unwrap();
    assert!(!mask.contains(BlindPixel::new(1, 1)));
    assert!(mask.contains(BlindPixel::new(2, 1)));
    assert!(!mask.contains(BlindPixel::new(8, 0)));
}
#[test]
fn reference_project_defaults_and_explicit_zero_differ() {
    let shape = Shape::new(1, 1, 1).unwrap();
    assert_eq!(PoolBoundary::default().resolved(32), (2, 2));
    assert_eq!(count(&pool(shape, 32, None, None)), 780);
    assert_eq!(count(&pool(shape, 32, Some(0), Some(0))), 1024);
    assert_eq!(count(&pool(shape, 8, Some(1), Some(0))), 36);
    assert_eq!(count(&pool(shape, 8, Some(1), Some(1))), 36); // radius 1 cuts nothing.
}
#[test]
fn erodes_union_not_shared_tile_seams_and_hugs_missing_cells() {
    let rectangle = pool(Shape::new(2, 1, 3).unwrap(), 8, Some(1), Some(0));
    assert_eq!(count(&rectangle), 14 * 6);
    let mask = pool_playable_mask(&rectangle).unwrap();
    assert!(mask.contains(BlindPixel::new(7, 4)));
    assert!(mask.contains(BlindPixel::new(8, 4)));
    let l_shape = pool(Shape::new(2, 2, 0b0111).unwrap(), 8, Some(1), Some(0));
    assert_eq!(count(&l_shape), 132);
    let mask = pool_playable_mask(&l_shape).unwrap();
    assert!(!mask.contains(BlindPixel::new(7, 7))); // diagonal adjacency to the hole counts.
    assert!(mask.contains(BlindPixel::new(6, 7)));
}
#[test]
fn invalid_geometry_is_not_reported_as_zero_sand() {
    let shape = Shape::new(1, 1, 1).unwrap();
    assert!(matches!(
        pool_playable_mask(&pool(shape, 8, Some(1), Some(4))),
        Err(PoolBoundaryError::CornerRadius { .. })
    ));
    assert_eq!(
        pool_playable_mask(&pool(shape, 8, Some(4), Some(0))).unwrap_err(),
        PoolBoundaryError::NoPlayablePixels
    );
}
