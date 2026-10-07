use oreak_core::{
    Blind, BlindGuide, BlindPixel, BlindTile, Block, DitherMode, EntityError, GridPoint,
    ImageArtError, ImagePlacement, ImageProjector, ImageSampling, ImageTransparency, IndexedImage,
    MAX_IMAGE_AXIS, MAX_PALETTE_MAPPINGS, PALETTE_HEX, PALETTE_RGB, PALETTE_VERSION,
    PaletteMapping, PaletteSettings, PlaceableEntity, Shape, SourceColorCluster, convert_rgba,
    extract_color_clusters, project_image,
};

fn image(width: u32, height: u32, pixels: Vec<u8>) -> IndexedImage {
    IndexedImage {
        width,
        height,
        pixels,
        palette_version: PALETTE_VERSION,
    }
}

fn placement() -> ImagePlacement {
    ImagePlacement {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
        sampling: ImageSampling::Nearest,
        pixelation: 1,
        resolution: None,
        transparency: ImageTransparency::Preserve,
    }
}

fn entity(shape: Shape, resolution: u8, colors: Vec<Vec<u8>>) -> PlaceableEntity {
    let tiles = colors
        .into_iter()
        .map(|colors| BlindTile::from_colors(resolution, colors).unwrap())
        .collect();
    PlaceableEntity::blind(
        "art",
        GridPoint::new(0, 0),
        shape,
        Blind::new(resolution, tiles).unwrap(),
    )
    .unwrap()
}

fn single(resolution: u8, color: u8) -> PlaceableEntity {
    entity(
        Shape::new(1, 1, 1).unwrap(),
        resolution,
        vec![vec![color; usize::from(resolution).pow(2)]],
    )
}

fn colors(entity: &PlaceableEntity) -> &[u8] {
    entity.as_blind().unwrap().tiles()[0].colors()
}

fn rgba_pixel(color: u8, alpha: u8) -> [u8; 4] {
    let [r, g, b] = PALETTE_RGB[usize::from(color) - 1];
    [r, g, b, alpha]
}

#[test]
fn image_projection_preserves_pool_groups_and_boundary_after_resampling() {
    let base = single(2, 0);
    let boundary = oreak_core::PoolBoundary {
        padding_pixels: Some(0),
        corner_radius_pixels: Some(0),
    };
    let blind = base
        .as_blind()
        .unwrap()
        .with_distribution_groups(
            base.shape(),
            vec![oreak_core::PoolDistributionGroup {
                id: 1,
                name: "Region".into(),
                pixels: vec![BlindPixel::new(0, 0)],
            }],
        )
        .unwrap()
        .with_boundary(boundary)
        .unwrap();
    let source =
        PlaceableEntity::blind(base.id().clone(), base.origin(), base.shape(), blind).unwrap();
    let expected = source.resampled_blind(4).unwrap();
    let placed = project_image(
        &image(1, 1, vec![3]),
        &PaletteSettings::default(),
        &ImagePlacement {
            resolution: Some(4),
            ..placement()
        },
        &source,
    )
    .unwrap();
    assert_eq!(placed.as_blind().unwrap().boundary(), boundary);
    assert_eq!(
        placed.as_blind().unwrap().distribution_groups(),
        expected.as_blind().unwrap().distribution_groups()
    );
    assert!(colors(&placed).iter().all(|color| *color == 3));
}

#[test]
fn fixed_palette_round_trips_exactly_without_resizing() {
    assert_eq!(
        PALETTE_RGB,
        [
            [255, 139, 104],
            [255, 209, 90],
            [114, 221, 145],
            [84, 202, 236],
            [120, 149, 255],
            [175, 130, 242],
            [241, 123, 210],
            [255, 116, 137],
            [182, 154, 123],
            [244, 247, 250],
        ]
    );
    for ([r, g, b], hex) in PALETTE_RGB.into_iter().zip(PALETTE_HEX) {
        assert_eq!(format!("#{r:02x}{g:02x}{b:02x}"), hex);
    }
    let rgba: Vec<_> = (1..=10).flat_map(|color| rgba_pixel(color, 255)).collect();
    let actual = convert_rgba(5, 2, &rgba, &PaletteSettings::default()).unwrap();
    assert_eq!(actual, image(5, 2, (1..=10).collect()));
}

#[test]
fn alpha_threshold_boundary_and_background_are_explicit() {
    let rgba: Vec<_> = [0, 127, 128, 255]
        .into_iter()
        .flat_map(|alpha| rgba_pixel(3, alpha))
        .collect();
    let settings = PaletteSettings::default();
    assert_eq!(
        convert_rgba(4, 1, &rgba, &settings).unwrap().pixels,
        [0, 0, 3, 3]
    );
    let settings = PaletteSettings {
        alpha_threshold: 0,
        ..settings
    };
    assert_eq!(
        convert_rgba(4, 1, &rgba, &settings).unwrap().pixels,
        [0, 3, 3, 3]
    );
    let settings = PaletteSettings {
        background: Some(2),
        alpha_threshold: 128,
        ..settings
    };
    let actual = convert_rgba(4, 1, &rgba, &settings).unwrap();
    assert_eq!(&actual.pixels[..2], &[2, 2]);
    assert_eq!(actual.pixels[3], 3);
    assert!(actual.pixels.iter().all(|&color| color != 0));
}

#[test]
fn enabled_palette_order_does_not_change_quantization_or_dither() {
    let rgba: Vec<_> = (0..256_u16)
        .flat_map(|v| [v as u8, v as u8, v as u8, 255])
        .collect();
    let settings = PaletteSettings {
        enabled_colors: vec![1, 5, 9, 10],
        dithering: DitherMode::FloydSteinberg,
        ..PaletteSettings::default()
    };
    let first = convert_rgba(32, 8, &rgba, &settings).unwrap();
    let mut reordered = settings.clone();
    reordered.enabled_colors.reverse();
    assert_eq!(first, convert_rgba(32, 8, &rgba, &reordered).unwrap());
    assert_eq!(first, convert_rgba(32, 8, &rgba, &settings).unwrap());
    assert!(
        first
            .pixels
            .iter()
            .all(|color| settings.enabled_colors.contains(color))
    );
    reordered.dithering = DitherMode::None;
    assert_ne!(first, convert_rgba(32, 8, &rgba, &reordered).unwrap());
}

#[test]
fn transparent_pixel_does_not_propagate_diffusion_error() {
    let settings = PaletteSettings {
        dithering: DitherMode::FloydSteinberg,
        ..PaletteSettings::default()
    };
    let actual = convert_rgba(
        3,
        1,
        &[150, 150, 150, 255, 255, 255, 255, 0, 150, 150, 150, 255],
        &settings,
    )
    .unwrap();
    assert_eq!(actual.pixels[1], 0);
    assert_eq!(actual.pixels[0], actual.pixels[2]);
}

#[test]
fn conversion_rejects_bad_dimensions_lengths_and_palette_settings() {
    let settings = PaletteSettings::default();
    for (width, height) in [(0, 1), (1, 0), (1025, 1), (1, 1025), (u32::MAX, u32::MAX)] {
        assert!(matches!(
            convert_rgba(width, height, &[], &settings),
            Err(ImageArtError::InvalidDimensions { .. })
        ));
    }
    assert!(matches!(
        convert_rgba(2, 1, &[0; 7], &settings),
        Err(ImageArtError::RgbaLength {
            expected: 8,
            actual: 7
        })
    ));
    assert!(matches!(
        convert_rgba(1, 1, &[0; 5], &settings),
        Err(ImageArtError::RgbaLength { .. })
    ));
    for enabled in [vec![], vec![0], vec![11], vec![1, 1]] {
        let settings = PaletteSettings {
            enabled_colors: enabled,
            ..PaletteSettings::default()
        };
        assert!(settings.validate().is_err());
        assert!(convert_rgba(1, 1, &[0; 4], &settings).is_err());
    }
    for background in [0, 2, 11, 255] {
        let settings = PaletteSettings {
            enabled_colors: vec![1],
            background: Some(background),
            ..PaletteSettings::default()
        };
        assert!(settings.validate().is_err());
    }
    assert!(
        convert_rgba(
            MAX_IMAGE_AXIS,
            1,
            &vec![255; MAX_IMAGE_AXIS as usize * 4],
            &settings
        )
        .is_ok()
    );
}

#[test]
fn maps_top_down_rows_to_cell_local_lower_left_without_flipping_cells() {
    let source = image(2, 4, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    let original = entity(
        Shape::new(1, 2, 0b11).unwrap(),
        2,
        vec![vec![9; 4], vec![9; 4]],
    )
    .moved_to(GridPoint::new(3, 4));
    let p = ImagePlacement {
        x: 3.0,
        y: 4.0,
        height: 2.0,
        ..placement()
    };
    let actual = project_image(&source, &PaletteSettings::default(), &p, &original).unwrap();
    assert_eq!(
        actual.as_blind().unwrap().tiles()[0].colors(),
        &[3, 4, 1, 2]
    );
    assert_eq!(
        actual.as_blind().unwrap().tiles()[1].colors(),
        &[7, 8, 5, 6]
    );
    assert_eq!(actual.id(), original.id());
    assert_eq!(actual.origin(), original.origin());
    assert_eq!(actual.shape(), original.shape());
    assert_eq!(colors(&original), &[9; 4]);
}

#[test]
fn clips_to_irregular_footprint_and_preserves_guides() {
    let shape = Shape::new(2, 2, 0b0111).unwrap();
    let guide = BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap();
    let original = PlaceableEntity::blind(
        "sparse",
        GridPoint::new(0, 0),
        shape,
        Blind::with_guides(1, vec![BlindTile::empty(1).unwrap(); 3], vec![guide]).unwrap(),
    )
    .unwrap();
    let p = ImagePlacement {
        width: 2.0,
        height: 2.0,
        ..placement()
    };
    let actual = project_image(
        &image(2, 2, vec![1, 2, 3, 4]),
        &PaletteSettings::default(),
        &p,
        &original,
    )
    .unwrap();
    let blind = actual.as_blind().unwrap();
    assert_eq!(
        blind
            .tiles()
            .iter()
            .map(|t| t.colors()[0])
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(blind.guides(), &[guide]);
    assert_eq!(actual.shape(), shape);
}

#[test]
fn only_centers_in_half_open_placement_are_changed() {
    let original = single(2, 9);
    let p = ImagePlacement {
        x: 0.25,
        y: 0.25,
        width: 0.5,
        height: 0.5,
        ..placement()
    };
    let actual = project_image(
        &image(1, 1, vec![1]),
        &PaletteSettings::default(),
        &p,
        &original,
    )
    .unwrap();
    // Center (0.25,0.25) is included; centers on right/bottom edge are excluded.
    assert_eq!(colors(&actual), &[9, 9, 1, 9]);
    let p = ImagePlacement {
        x: -0.5,
        y: -0.5,
        ..placement()
    };
    let clipped = project_image(
        &image(1, 1, vec![2]),
        &PaletteSettings::default(),
        &p,
        &original,
    )
    .unwrap();
    assert_eq!(colors(&clipped), &[9, 9, 2, 9]);
    let p = ImagePlacement {
        x: 10.0,
        ..placement()
    };
    assert_eq!(
        project_image(
            &image(1, 1, vec![1]),
            &PaletteSettings::default(),
            &p,
            &original
        )
        .unwrap(),
        original
    );
}

#[test]
fn transparency_preserves_or_erases_only_intersecting_pixels() {
    let original = single(2, 9);
    let source = image(2, 2, vec![0, 1, 2, 0]);
    let preserve = project_image(
        &source,
        &PaletteSettings::default(),
        &placement(),
        &original,
    )
    .unwrap();
    assert_eq!(colors(&preserve), &[2, 9, 9, 1]);
    let p = ImagePlacement {
        transparency: ImageTransparency::Erase,
        ..placement()
    };
    let erase = project_image(&source, &PaletteSettings::default(), &p, &original).unwrap();
    assert_eq!(colors(&erase), &[2, 0, 0, 1]);
}

#[test]
fn resolution_resamples_existing_pixels_and_guides_before_projection() {
    let shape = Shape::new(1, 1, 1).unwrap();
    let guide = BlindGuide::new(BlindPixel::new(0, 0), BlindPixel::new(1, 0)).unwrap();
    let original = PlaceableEntity::blind(
        "guide",
        GridPoint::new(0, 0),
        shape,
        Blind::with_guides(
            2,
            vec![BlindTile::from_colors(2, vec![1, 2, 3, 4]).unwrap()],
            vec![guide],
        )
        .unwrap(),
    )
    .unwrap();
    let baseline = original.resampled_blind(4).unwrap();
    let p = ImagePlacement {
        width: 0.25,
        height: 0.25,
        resolution: Some(4),
        ..placement()
    };
    let actual = project_image(
        &image(1, 1, vec![5]),
        &PaletteSettings::default(),
        &p,
        &original,
    )
    .unwrap();
    let mut expected = colors(&baseline).to_vec();
    expected[12] = 5;
    assert_eq!(colors(&actual), expected);
    assert_eq!(
        actual.as_blind().unwrap().guides(),
        baseline.as_blind().unwrap().guides()
    );
    let offscreen = ImagePlacement {
        x: 100.0,
        resolution: Some(4),
        ..placement()
    };
    assert_eq!(
        project_image(
            &image(1, 1, vec![5]),
            &PaletteSettings::default(),
            &offscreen,
            &original
        )
        .unwrap(),
        baseline
    );
}

#[test]
fn pixelation_is_source_aligned_and_independent_of_destination_resolution() {
    let source = image(4, 1, vec![1, 2, 3, 4]);
    let p = ImagePlacement {
        pixelation: 2,
        ..placement()
    };
    let actual = project_image(&source, &PaletteSettings::default(), &p, &single(4, 0)).unwrap();
    for row in colors(&actual).chunks(4) {
        assert_eq!(row, &[1, 1, 3, 3]);
    }
    let area = ImagePlacement {
        sampling: ImageSampling::Area,
        ..p.clone()
    };
    let actual = project_image(&source, &PaletteSettings::default(), &area, &single(4, 0)).unwrap();
    for row in colors(&actual).chunks(4) {
        assert_eq!(row, &[1, 1, 3, 3]);
    }
    let crop = ImagePlacement {
        x: -0.5,
        width: 2.0,
        ..p
    };
    let actual = project_image(&source, &PaletteSettings::default(), &crop, &single(4, 0)).unwrap();
    for row in colors(&actual).chunks(4) {
        assert_eq!(row, &[1, 1, 3, 3]);
    }
}

#[test]
fn nearest_requantizes_disabled_colors_without_touching_outside_pixels() {
    let settings = PaletteSettings {
        enabled_colors: vec![2],
        ..PaletteSettings::default()
    };
    let p = ImagePlacement {
        width: 0.5,
        ..placement()
    };
    let actual = project_image(&image(1, 1, vec![5]), &settings, &p, &single(2, 9)).unwrap();
    assert_eq!(colors(&actual), &[2, 9, 2, 9]);
}

#[test]
fn area_averages_palette_rgb_not_numeric_indices_and_requantizes() {
    let settings = PaletteSettings::default();
    let p = ImagePlacement {
        sampling: ImageSampling::Area,
        ..placement()
    };
    let actual = project_image(&image(2, 1, vec![1, 10]), &settings, &p, &single(1, 0)).unwrap();
    let a = PALETTE_RGB[0];
    let b = PALETTE_RGB[9];
    let mut rgba = [0; 4];
    for c in 0..3 {
        rgba[c] = ((u16::from(a[c]) + u16::from(b[c])) / 2) as u8;
    }
    rgba[3] = 255;
    let expected = convert_rgba(1, 1, &rgba, &settings).unwrap().pixels[0];
    assert_eq!(colors(&actual), &[expected]);
    assert_ne!(expected, 10); // Nearest would select the right source pixel.
    let settings = PaletteSettings {
        enabled_colors: vec![5],
        ..settings
    };
    assert_eq!(
        colors(&project_image(&image(2, 1, vec![1, 10]), &settings, &p, &single(1, 0)).unwrap()),
        &[5]
    );
}

#[test]
fn area_respects_fractional_bounds_and_transparent_coverage() {
    let source = image(2, 1, vec![0, 3]);
    let p = ImagePlacement {
        sampling: ImageSampling::Area,
        ..placement()
    };
    let settings = PaletteSettings::default();
    // Exactly 50% coverage is below the default 128/255 threshold.
    assert_eq!(
        colors(&project_image(&source, &settings, &p, &single(1, 9)).unwrap()),
        &[9]
    );
    let erase = ImagePlacement {
        transparency: ImageTransparency::Erase,
        ..p.clone()
    };
    assert_eq!(
        colors(&project_image(&source, &settings, &erase, &single(1, 9)).unwrap()),
        &[0]
    );
    let settings = PaletteSettings {
        alpha_threshold: 127,
        ..settings
    };
    assert_eq!(
        colors(&project_image(&source, &settings, &p, &single(1, 9)).unwrap()),
        &[3]
    );
    // Source bounds [0.5,1.5] cover exactly half of each source pixel, too.
    let fractional = ImagePlacement {
        x: -0.5,
        width: 2.0,
        ..p
    };
    assert_eq!(
        colors(&project_image(&source, &settings, &fractional, &single(1, 9)).unwrap()),
        &[3]
    );
}

#[test]
fn area_keeps_opaque_pixels_at_extreme_valid_scales() {
    let source = image(16, 16, vec![3; 256]);
    let settings = PaletteSettings {
        alpha_threshold: 255,
        ..PaletteSettings::default()
    };
    for scale in [1e5, 1e10, 1e15, 1e300] {
        let p = ImagePlacement {
            x: -scale / 2.0,
            y: -scale / 2.0,
            width: scale,
            height: scale,
            sampling: ImageSampling::Area,
            ..placement()
        };
        let actual = project_image(&source, &settings, &p, &single(2, 9)).unwrap();
        assert_eq!(colors(&actual), &[3; 4], "scale={scale}");
    }
}

#[test]
fn prepared_maximum_image_reuses_one_context_for_many_targets_and_pointer_moves() {
    // Regression workload: preparation is O(1024^2) once, not once per Pool
    // or pointer move. Each pass below only visits 64 destination pixels.
    let source = image(1024, 1024, vec![4; 1024 * 1024]);
    let settings = PaletteSettings::default();
    let projector = ImageProjector::new(&source, &settings, ImageSampling::Area, 1).unwrap();
    let targets: Vec<_> = (0..64)
        .map(|index| single(1, 0).moved_to(GridPoint::new(index % 8, index / 8)))
        .collect();
    for offset in [0.0, -0.1, -0.2, -0.3, -0.4, 0.0] {
        let p = ImagePlacement {
            x: offset,
            y: offset,
            width: 8.0,
            height: 8.0,
            sampling: ImageSampling::Area,
            ..placement()
        };
        for target in &targets {
            let actual = projector.project(&p, target).unwrap();
            assert_eq!(colors(&actual), &[4]);
            assert_eq!(actual.id(), target.id());
            assert_eq!(actual.origin(), target.origin());
        }
    }
    assert!(projector.matches_source(&source, &settings, ImageSampling::Area, 1));
}

#[test]
fn prepared_projection_matches_wrapper_and_owns_immutable_source() {
    let mut source = image(2, 2, vec![1, 0, 5, 10]);
    let settings = PaletteSettings::default();
    for sampling in [ImageSampling::Nearest, ImageSampling::Area] {
        let projector = ImageProjector::new(&source, &settings, sampling, 1).unwrap();
        for resolution in [1, 2, 3, 4] {
            let p = ImagePlacement {
                sampling,
                resolution: Some(resolution),
                transparency: ImageTransparency::Erase,
                ..placement()
            };
            assert_eq!(
                projector.project(&p, &single(2, 9)).unwrap(),
                project_image(&source, &settings, &p, &single(2, 9)).unwrap()
            );
        }
    }
    let projector = ImageProjector::new(&source, &settings, ImageSampling::Nearest, 1).unwrap();
    let expected = projector.project(&placement(), &single(2, 9)).unwrap();
    source.pixels.fill(2);
    assert!(!projector.matches_source(&source, &settings, ImageSampling::Nearest, 1));
    assert_eq!(
        projector.project(&placement(), &single(2, 9)).unwrap(),
        expected
    );
}

#[test]
fn prepared_context_validates_source_and_rejects_stale_sampling_or_pixelation() {
    let source = image(1, 1, vec![1]);
    let settings = PaletteSettings::default();
    let projector = ImageProjector::new(&source, &settings, ImageSampling::Nearest, 1).unwrap();
    assert!(projector.matches_source(&source, &settings, ImageSampling::Nearest, 1));
    assert!(!projector.matches_source(&source, &settings, ImageSampling::Area, 1));
    assert!(!projector.matches_source(&source, &settings, ImageSampling::Nearest, 2));
    let altered = PaletteSettings {
        alpha_threshold: 0,
        ..settings.clone()
    };
    assert!(!projector.matches_source(&source, &altered, ImageSampling::Nearest, 1));
    for p in [
        ImagePlacement {
            sampling: ImageSampling::Area,
            ..placement()
        },
        ImagePlacement {
            pixelation: 2,
            ..placement()
        },
    ] {
        assert_eq!(
            projector.project(&p, &single(1, 9)),
            Err(ImageArtError::ProjectorSettingsMismatch)
        );
    }
    assert!(
        ImageProjector::new(&image(1, 1, vec![11]), &settings, ImageSampling::Area, 1).is_err()
    );
    assert!(
        ImageProjector::new(
            &source,
            &PaletteSettings {
                enabled_colors: vec![],
                ..settings.clone()
            },
            ImageSampling::Area,
            1
        )
        .is_err()
    );
    for pixelation in [0, 1025, u16::MAX] {
        assert!(ImageProjector::new(&source, &settings, ImageSampling::Area, pixelation).is_err());
    }
}

#[test]
fn maximum_source_area_sampling_is_bounded_and_deterministic() {
    let source = image(1024, 1024, vec![4; 1024 * 1024]);
    let p = ImagePlacement {
        sampling: ImageSampling::Area,
        resolution: Some(32),
        ..placement()
    };
    let actual = project_image(&source, &PaletteSettings::default(), &p, &single(1, 0)).unwrap();
    assert_eq!(colors(&actual), &[4; 1024]);
    assert_eq!(
        actual,
        project_image(&source, &PaletteSettings::default(), &p, &single(1, 0)).unwrap()
    );
}

#[test]
fn validates_indexed_data_placement_and_entity_before_projection() {
    let settings = PaletteSettings::default();
    let original = single(1, 1);
    for source in [
        image(0, 1, vec![]),
        image(1, 1, vec![]),
        image(1, 1, vec![11]),
        IndexedImage {
            palette_version: 2,
            ..image(1, 1, vec![1])
        },
    ] {
        assert!(source.validate().is_err());
        assert!(project_image(&source, &settings, &placement(), &original).is_err());
    }
    let source = image(1, 1, vec![1]);
    for p in [
        ImagePlacement {
            x: f64::NAN,
            ..placement()
        },
        ImagePlacement {
            y: f64::INFINITY,
            ..placement()
        },
        ImagePlacement {
            width: f64::NEG_INFINITY,
            ..placement()
        },
        ImagePlacement {
            height: 0.0,
            ..placement()
        },
        ImagePlacement {
            width: -1.0,
            ..placement()
        },
        ImagePlacement {
            x: f64::MAX,
            width: f64::MAX,
            ..placement()
        },
        ImagePlacement {
            x: 1.0,
            width: f64::MIN_POSITIVE,
            ..placement()
        },
        ImagePlacement {
            pixelation: 0,
            ..placement()
        },
        ImagePlacement {
            pixelation: 1025,
            ..placement()
        },
        ImagePlacement {
            resolution: Some(0),
            ..placement()
        },
        ImagePlacement {
            resolution: Some(33),
            ..placement()
        },
    ] {
        assert!(p.validate().is_err(), "{p:?}");
        assert!(project_image(&source, &settings, &p, &original).is_err());
        assert_eq!(p, p.clone()); // Total float equality is reflexive even for NaN.
    }
    let block = PlaceableEntity::block(
        "block",
        GridPoint::new(0, 0),
        Shape::new(1, 1, 1).unwrap(),
        Block::default(),
    )
    .unwrap();
    assert!(matches!(
        project_image(&source, &settings, &placement(), &block),
        Err(ImageArtError::Entity(EntityError::NotBlind(_)))
    ));
}

#[test]
fn palette_mappings_validate_bounds_unique_buckets_and_enabled_targets() {
    let mut settings = PaletteSettings {
        enabled_colors: vec![1, 2],
        mappings: vec![PaletteMapping {
            source_rgb: [8, 8, 8],
            target_color: 2,
        }],
        ..PaletteSettings::default()
    };
    assert!(settings.validate().is_ok());

    settings.mappings.push(PaletteMapping {
        source_rgb: [15, 15, 15],
        target_color: 1,
    });
    assert!(matches!(
        settings.validate(),
        Err(ImageArtError::DuplicateMappingSource([15, 15, 15]))
    ));
    settings.mappings = vec![PaletteMapping {
        source_rgb: [0, 0, 0],
        target_color: 3,
    }];
    assert!(matches!(
        settings.validate(),
        Err(ImageArtError::InvalidMappingTarget(3))
    ));
    settings.mappings = (0..=MAX_PALETTE_MAPPINGS)
        .map(|index| PaletteMapping {
            source_rgb: [
                ((index & 31) << 3) as u8,
                (((index >> 5) & 31) << 3) as u8,
                0,
            ],
            target_color: 1,
        })
        .collect();
    assert!(matches!(
        settings.validate(),
        Err(ImageArtError::TooManyMappings(_))
    ));
}

#[test]
fn mapped_bucket_overrides_default_and_unmapped_bucket_stays_automatic() {
    let mapped = PALETTE_RGB[0];
    let unmapped = PALETTE_RGB[2];
    let rgba = [
        mapped[0],
        mapped[1],
        mapped[2],
        255,
        unmapped[0],
        unmapped[1],
        unmapped[2],
        255,
    ];
    let settings = PaletteSettings {
        mappings: vec![PaletteMapping {
            source_rgb: mapped,
            target_color: 5,
        }],
        ..PaletteSettings::default()
    };
    assert_eq!(convert_rgba(2, 1, &rgba, &settings).unwrap().pixels, [5, 3]);
}

#[test]
fn source_mapping_survives_background_but_does_not_recolor_transparency() {
    let rgba = [0, 0, 0, 128, 0, 0, 0, 0, 0, 0, 0, 127];
    let cluster = extract_color_clusters(3, 1, &rgba, 1).unwrap()[0];
    let settings = PaletteSettings {
        background: Some(10),
        mappings: vec![PaletteMapping {
            source_rgb: cluster.rgb,
            target_color: 1,
        }],
        ..PaletteSettings::default()
    };
    assert_eq!(
        convert_rgba(3, 1, &rgba, &settings).unwrap().pixels,
        [1, 10, 10]
    );
    let without_background = PaletteSettings {
        background: None,
        ..settings
    };
    assert_eq!(
        convert_rgba(3, 1, &rgba, &without_background)
            .unwrap()
            .pixels,
        [1, 0, 0]
    );
}

#[test]
fn mapped_dithering_is_deterministic_and_diffuses_against_target() {
    let source = [128, 128, 128];
    let rgba: Vec<_> = (0..64)
        .flat_map(|_| [source[0], source[1], source[2], 255])
        .collect();
    let settings = PaletteSettings {
        dithering: DitherMode::FloydSteinberg,
        mappings: vec![PaletteMapping {
            source_rgb: source,
            target_color: 1,
        }],
        ..PaletteSettings::default()
    };
    let first = convert_rgba(8, 8, &rgba, &settings).unwrap();
    assert_eq!(first, convert_rgba(8, 8, &rgba, &settings).unwrap());
    assert!(first.pixels.iter().all(|&color| color == 1));
    let without_dither = convert_rgba(
        8,
        8,
        &rgba,
        &PaletteSettings {
            dithering: DitherMode::None,
            ..settings
        },
    )
    .unwrap();
    assert_eq!(first, without_dither);
}

#[test]
fn extracts_dominant_clusters_and_ignores_only_fully_transparent_pixels() {
    let rgba = [
        8, 8, 8, 255, 15, 15, 15, 1, 8, 8, 8, 255, 200, 200, 200, 255, 0, 0, 0, 0,
    ];
    assert_eq!(
        extract_color_clusters(5, 1, &rgba, 2).unwrap(),
        vec![
            SourceColorCluster {
                rgb: [10, 10, 10],
                pixel_count: 3,
            },
            SourceColorCluster {
                rgb: [200, 200, 200],
                pixel_count: 1,
            },
        ]
    );
    assert_eq!(
        extract_color_clusters(1, 1, &[0, 0, 0, 0], 1).unwrap(),
        vec![]
    );
}

#[test]
fn cluster_extraction_validates_dimensions_length_and_limit() {
    for (width, height) in [(0, 1), (1, 0), (MAX_IMAGE_AXIS + 1, 1)] {
        assert!(matches!(
            extract_color_clusters(width, height, &[], 1),
            Err(ImageArtError::InvalidDimensions { .. })
        ));
    }
    assert!(matches!(
        extract_color_clusters(1, 1, &[0; 3], 1),
        Err(ImageArtError::RgbaLength { .. })
    ));
    for limit in [0, MAX_PALETTE_MAPPINGS + 1] {
        assert_eq!(
            extract_color_clusters(1, 1, &[0; 4], limit),
            Err(ImageArtError::InvalidMaxClusters(limit))
        );
    }
}

#[test]
fn palette_settings_serde_defaults_and_omits_empty_mappings() {
    let json =
        r#"{"enabled_colors":[1,2],"dithering":"none","alpha_threshold":128,"background":null}"#;
    let settings: PaletteSettings = serde_json::from_str(json).unwrap();
    assert!(settings.mappings.is_empty());
    assert_eq!(serde_json::to_string(&settings).unwrap(), json);
}

#[test]
fn serde_round_trips_with_snake_case_and_rejects_unknown_fields() {
    let settings = PaletteSettings {
        dithering: DitherMode::FloydSteinberg,
        ..PaletteSettings::default()
    };
    let value = serde_json::to_value(&settings).unwrap();
    assert_eq!(value["dithering"], "floyd_steinberg");
    assert_eq!(
        serde_json::from_value::<PaletteSettings>(value.clone()).unwrap(),
        settings
    );
    let source = image(1, 1, vec![3]);
    assert_eq!(
        serde_json::from_str::<IndexedImage>(&serde_json::to_string(&source).unwrap()).unwrap(),
        source
    );
    let p = ImagePlacement {
        sampling: ImageSampling::Area,
        transparency: ImageTransparency::Erase,
        ..placement()
    };
    let value = serde_json::to_value(&p).unwrap();
    assert_eq!(value["sampling"], "area");
    assert_eq!(value["transparency"], "erase");
    assert_eq!(serde_json::from_value::<ImagePlacement>(value).unwrap(), p);
    let mut v = serde_json::to_value(&settings).unwrap();
    v["extra"] = true.into();
    assert!(serde_json::from_value::<PaletteSettings>(v).is_err());
    let mut v = serde_json::to_value(&source).unwrap();
    v["extra"] = true.into();
    assert!(serde_json::from_value::<IndexedImage>(v).is_err());
    let mut v = serde_json::to_value(&p).unwrap();
    v["extra"] = true.into();
    assert!(serde_json::from_value::<ImagePlacement>(v).is_err());
    let mut v = serde_json::to_value(&p).unwrap();
    v["width"] = 0.into();
    assert!(serde_json::from_value::<ImagePlacement>(v).is_err());
}
