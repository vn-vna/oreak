use oreak_core::{
    Blind, BlindPixel, BlindTile, GridPoint, LevelSnapshot, PlaceableEntity, PlaceableEntityKind,
    PoolBoundary, PoolDistributionGroup,
};
use oreak_legacy::{DATA_CODEC_SALT_SIZE, DataCodec, LegacyError, LegacyLevel};
use serde_json::{Value, json};

const SALT: [u8; DATA_CODEC_SALT_SIZE] = [0, 1, 2, 3, 4, 5, 6, 7];

fn fixture(fields: &str, placeholder: bool) -> String {
    let board = DataCodec::encode_with_salt(&[0x0f], &SALT, false).unwrap();
    let shape = DataCodec::encode_with_salt(&[0x01], &SALT, false).unwrap();
    let canvas = if placeholder {
        r#"{ "r" : [1,1], "cmp":true, "data":null, "opaque":[3,  4] }"#.to_owned()
    } else {
        let pixels = DataCodec::encode_with_salt(&[0x1a, 0, 0x2b, 0], &SALT, true).unwrap();
        format!(r#"{{ "r" : [2,2], "cmp":true, "data":"{pixels}", "opaque":[3,  4] }}"#)
    };
    format!(
        r#"{{ "dur":1e0, "bes":[2,2], "bdat":"{board}", "future":[1,  2], "entitites":[["pool",{{"eid":"p","future":{{"raw":true}},"g":{{"r":[0,0,1,1],"d":"{shape}","opaque":"grid"}},"stc":{{{fields}"spauth":{{"unknown":[null,  0,"opaque"],"number":1e0}},"future":[7,  8],"c":{canvas}}}}}]] }}"#
    )
}

fn blind(level: &LegacyLevel) -> &Blind {
    let PlaceableEntityKind::Blind(blind) = level.snapshot().entities()[0].kind() else {
        panic!("expected pool");
    };
    blind
}

fn snapshot_with_blind(level: &LegacyLevel, blind: Blind) -> LevelSnapshot {
    let original = &level.snapshot().entities()[0];
    let replacement = PlaceableEntity::blind(
        original.id().clone(),
        original.origin(),
        original.shape(),
        blind,
    )
    .unwrap();
    LevelSnapshot::from_parts(
        level.snapshot().size(),
        level.snapshot().cells().to_vec(),
        vec![replacement],
        level.snapshot().decorators().to_vec(),
    )
    .unwrap()
}

#[test]
fn import_absent_null_zero_and_nonzero_boundaries_roundtrips_exactly() {
    let cases = [
        (None, None),
        (Some("null"), None),
        (Some("0"), Some(0)),
        (Some("17"), Some(17)),
        (Some("1024"), Some(1024)),
    ];
    for (padding_wire, padding_pixels) in cases {
        for (radius_wire, corner_radius_pixels) in cases {
            let mut fields = String::new();
            if let Some(value) = padding_wire {
                fields.push_str(&format!(r#""spp":{value},"#));
            }
            if let Some(value) = radius_wire {
                fields.push_str(&format!(r#""spcr":{value},"#));
            }
            for placeholder in [false, true] {
                let source = fixture(&fields, placeholder);
                let level = LegacyLevel::parse(&source).unwrap();
                assert_eq!(
                    blind(&level).boundary(),
                    PoolBoundary {
                        padding_pixels,
                        corner_radius_pixels,
                    }
                );
                assert_eq!(level.export(level.snapshot()).unwrap(), source);
            }
        }
    }
}

#[test]
fn malformed_boundary_values_have_field_specific_diagnostics() {
    for field in ["spp", "spcr"] {
        for value in [
            "-1",
            "0.5",
            "1.0",
            "1e0",
            "\"0\"",
            "true",
            "[]",
            "{}",
            "1025",
            "65536",
            "18446744073709551616",
        ] {
            let source = fixture(&format!(r#""{field}":{value},"#), false);
            assert!(
                matches!(
                    LegacyLevel::parse(&source),
                    Err(LegacyError::InvalidField { path, message })
                        if path == format!("entitites[0][1].stc.{field}")
                            && message.contains("0..=1024")
                ),
                "accepted or misdiagnosed {field}={value}"
            );
        }
    }
}

#[test]
fn boundary_only_patches_preserve_every_unrelated_payload_value_and_canvas_token() {
    for placeholder in [false, true] {
        let source = fixture(r#""spp":0,"spcr":9,"#, placeholder);
        let level = LegacyLevel::parse(&source).unwrap();
        let boundary = PoolBoundary {
            padding_pixels: Some(23),
            corner_radius_pixels: None,
        };
        let changed = snapshot_with_blind(&level, blind(&level).with_boundary(boundary).unwrap());
        let output = level.export(&changed).unwrap();
        let mut expected: Value = serde_json::from_str(&source).unwrap();
        expected["entitites"][0][1]["stc"]["spp"] = json!(23);
        expected["entitites"][0][1]["stc"]["spcr"] = Value::Null;
        assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), expected);
        assert!(output.contains(r#""spauth":{"unknown":[null,  0,"opaque"],"number":1e0}"#));
        assert!(output.contains(r#""opaque":[3,  4]"#));
        assert_eq!(LegacyLevel::parse(&output).unwrap().snapshot(), &changed);
    }
}

#[test]
fn changing_one_boundary_field_preserves_absence_null_or_zero_of_the_other() {
    for (changed_field, other_field) in [("spp", "spcr"), ("spcr", "spp")] {
        for other in [None, Some("null"), Some("0"), Some("15")] {
            let fields = other
                .map(|value| format!(r#""{other_field}":{value},"#))
                .unwrap_or_default();
            let source = fixture(&fields, false);
            let level = LegacyLevel::parse(&source).unwrap();
            let mut boundary = blind(&level).boundary();
            if changed_field == "spp" {
                boundary.padding_pixels = Some(0);
            } else {
                boundary.corner_radius_pixels = Some(0);
            }
            let changed =
                snapshot_with_blind(&level, blind(&level).with_boundary(boundary).unwrap());
            let output: Value = serde_json::from_str(&level.export(&changed).unwrap()).unwrap();
            let mut expected: Value = serde_json::from_str(&source).unwrap();
            expected["entitites"][0][1]["stc"][changed_field] = json!(0);
            assert_eq!(output, expected);
            assert_eq!(
                output["entitites"][0][1]["stc"].get(other_field).is_some(),
                other.is_some()
            );
        }
    }
}

#[test]
fn pixel_edits_preserve_boundary_fields_and_opaque_authoring_data() {
    for fields in ["", r#""spp":null,"spcr":0,"#, r#""spp":7,"spcr":null,"#] {
        let source = fixture(fields, false);
        let level = LegacyLevel::parse(&source).unwrap();
        let replacement = Blind::new(
            2,
            vec![BlindTile::from_colors(2, vec![2, 3, 1, 0]).unwrap()],
        )
        .unwrap()
        .with_boundary(blind(&level).boundary())
        .unwrap();
        let changed = snapshot_with_blind(&level, replacement);
        let output = level.export(&changed).unwrap();
        let mut actual: Value = serde_json::from_str(&output).unwrap();
        let expected: Value = serde_json::from_str(&source).unwrap();
        assert_ne!(
            actual["entitites"][0][1]["stc"]["c"]["data"],
            expected["entitites"][0][1]["stc"]["c"]["data"]
        );
        actual["entitites"][0][1]["stc"]["c"]["data"] =
            expected["entitites"][0][1]["stc"]["c"]["data"].clone();
        assert_eq!(actual, expected);
        assert_eq!(LegacyLevel::parse(&output).unwrap().snapshot(), &changed);
    }
}

#[test]
fn newly_created_pools_export_explicit_boundaries_without_inventing_defaults() {
    let template = LegacyLevel::parse(&fixture("", false)).unwrap();
    let mut empty: Value = serde_json::from_str(&fixture("", false)).unwrap();
    empty["entitites"] = json!([]);
    let level = LegacyLevel::parse(&empty.to_string()).unwrap();
    for boundary in [
        PoolBoundary::default(),
        PoolBoundary {
            padding_pixels: Some(0),
            corner_radius_pixels: Some(1024),
        },
    ] {
        let entity = PlaceableEntity::blind(
            "new",
            GridPoint::new(0, 0),
            template.snapshot().entities()[0].shape(),
            blind(&template).with_boundary(boundary).unwrap(),
        )
        .unwrap();
        let changed = LevelSnapshot::from_parts(
            level.snapshot().size(),
            level.snapshot().cells().to_vec(),
            vec![entity],
            vec![],
        )
        .unwrap();
        let output = level.export(&changed).unwrap();
        assert_eq!(LegacyLevel::parse(&output).unwrap().snapshot(), &changed);
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            value["entitites"][0][1]["stc"].get("spp").is_some(),
            boundary.padding_pixels.is_some()
        );
        assert_eq!(
            value["entitites"][0][1]["stc"].get("spcr").is_some(),
            boundary.corner_radius_pixels.is_some()
        );
    }
}

#[test]
fn editor_only_distribution_masks_never_emit_guides_or_rewrite_canvas() {
    let source = fixture(r#""spp":null,"spcr":0,"#, false);
    let level = LegacyLevel::parse(&source).unwrap();
    let grouped = blind(&level)
        .with_distribution_groups(
            level.snapshot().entities()[0].shape(),
            vec![PoolDistributionGroup {
                id: 1,
                name: "Custom region".to_owned(),
                pixels: vec![BlindPixel::new(0, 0)],
            }],
        )
        .unwrap();
    assert!(grouped.guides().is_empty());
    let changed = snapshot_with_blind(&level, grouped);
    let output = level.export(&changed).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap(),
        serde_json::from_str::<Value>(&source).unwrap()
    );
    let imported = LegacyLevel::parse(&output).unwrap();
    assert!(blind(&imported).guides().is_empty());
    assert!(blind(&imported).distribution_groups().is_empty());
    assert_eq!(imported.snapshot(), level.snapshot());
}
