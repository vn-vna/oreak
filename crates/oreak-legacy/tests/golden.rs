use oreak_core::{
    Blind, BlindTile, CardinalDirection, CollectCapacity, CommandEnvelope, CommandMetadata,
    DecoratorId, DecoratorKind, EntityId, GridPoint, LevelCommand, LevelSnapshot, LevelTimeline,
    PlaceableEntity, PlaceableEntityKind,
};
use oreak_legacy::{
    CodecError, CompatibilityState, DATA_CODEC_SALT_SIZE, DISABLED_COLLECT_COLOR_INDEX, DataCodec,
    LegacyError, LegacyLevel, UnknownEntityCodec, resolved_collect_radius,
};
use pretty_assertions::assert_eq;

const SALT: [u8; DATA_CODEC_SALT_SIZE] = [0, 1, 2, 3, 4, 5, 6, 7];

fn token(data: &[u8]) -> String {
    DataCodec::encode_with_salt(data, &SALT, false).unwrap()
}

fn gzip_token(data: &[u8]) -> String {
    DataCodec::encode_with_salt(data, &SALT, true).unwrap()
}

fn minimal_json() -> String {
    format!(
        r#"{{ "dur" : 1e0, "bes" : [ 2, 2 ], "bdat" : "{}", "entitites" : [], "future" : {{"raw":true}} }}"#,
        token(&[0x0f])
    )
}

#[test]
fn data_codec_matches_the_runtime_crc_and_repeated_salt_xor() {
    let encoded = DataCodec::encode_with_salt(&[5], &SALT, false).unwrap();
    assert_eq!(encoded, "AAECAwQFBgc=:BQ==:C2503FBF");
    assert_eq!(DataCodec::decode(&encoded).unwrap().data, vec![5]);

    let long: Vec<_> = (0..25).collect();
    let encoded = DataCodec::encode_with_salt(&long, &SALT, false).unwrap();
    assert_eq!(DataCodec::decode(&encoded).unwrap().data, long);
    let compressed = DataCodec::encode_with_salt(&long, &SALT, true).unwrap();
    assert_eq!(
        DataCodec::decode_gzip(&compressed, long.len())
            .unwrap()
            .data,
        long
    );
}

#[test]
fn minimal_level_is_lossless_and_json_is_strict() {
    let json = minimal_json();
    let level = LegacyLevel::parse(&json).unwrap();
    assert_eq!(level.duration(), 1.0);
    assert_eq!(level.snapshot().size().cell_count(), 4);
    assert_eq!(level.export(level.snapshot()).unwrap(), json);

    assert!(matches!(
        LegacyLevel::parse(r#"{"dur":0,"dur":1}"#),
        Err(LegacyError::Json { message, .. }) if message.contains("duplicate")
    ));
    assert!(matches!(
        LegacyLevel::parse(r#"{"dur":0,"x":{"a":1,"a":2}}"#),
        Err(LegacyError::Json { message, .. }) if message.contains("duplicate")
    ));
    assert!(matches!(
        LegacyLevel::parse("{} {}"),
        Err(LegacyError::Json { message, .. }) if message.contains("additional")
    ));
}

#[test]
fn asymmetric_shape_rows_and_unknown_payload_tokens_survive_a_root_patch() {
    let board = token(&[0xff, 0xff]);
    let shape = token(&[0x07]);
    let json = format!(
        r#"{{"dur":1e0,"bes":[4,4],"bdat":"{board}","rootFuture":[1,  2],"entitites":[["block",{{"eid":"b","future":{{"z":true}},"g":{{"r":[1,1,2,2],"d":"{shape}","opaque":"keep"}},"cc":[[3,null,9,4.5,"tail"]]}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let entity = &level.snapshot().entities()[0];
    assert_eq!(entity.shape().occupied_mask(), 0b0111);
    let PlaceableEntityKind::Block(block) = entity.kind() else {
        panic!("expected Block");
    };
    assert_eq!(resolved_collect_radius(&block.collect_layers()[0]), 20);
    assert_eq!(block.collect_layers()[0].radius(), None);

    let output = level.export_with_duration(level.snapshot(), 2.0).unwrap();
    assert!(output.contains(r#""rootFuture":[1,  2]"#));
    assert!(output.contains(r#""future":{"z":true}"#));
    assert!(output.contains(r#""opaque":"keep""#));
    assert!(output.contains(r#""cc":[[3,null,9,4.5,"tail"]]"#));
    assert!(output.contains(&format!(r#""d":"{shape}""#)));
    assert_eq!(
        LegacyLevel::parse(&output).unwrap().snapshot(),
        level.snapshot()
    );
}

#[test]
fn blind_canvas_uses_top_down_wire_rows_and_lower_left_core_tiles() {
    let board = token(&[0xff]);
    let shape = token(&[0x03]);
    let wire = [0x1a, 0, 0x2b, 0, 0, 0x3c, 0, 0x4d];
    let canvas = gzip_token(&wire);
    let json = format!(
        r#"{{"dur":0,"bes":[4,2],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,2,1],"d":"{shape}"}},"stc":{{"future":9,"c":{{"r":[4,2],"cmp":true,"data":"{canvas}","opaque":"yes"}}}}}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let PlaceableEntityKind::Blind(blind) = level.snapshot().entities()[0].kind() else {
        panic!("expected Blind");
    };
    assert_eq!(blind.tiles()[0].colors(), &[0, 3, 1, 0]);
    assert_eq!(blind.tiles()[1].colors(), &[0, 4, 2, 0]);

    let mut first = blind.tiles()[0].colors().to_vec();
    first[2] = 5;
    let changed_blind = Blind::new(
        2,
        vec![
            BlindTile::from_colors(2, first).unwrap(),
            blind.tiles()[1].clone(),
        ],
    )
    .unwrap();
    let replacement = PlaceableEntity::blind(
        "p",
        GridPoint::new(0, 0),
        level.snapshot().entities()[0].shape(),
        changed_blind,
    )
    .unwrap();
    let changed = LevelSnapshot::from_parts(
        level.snapshot().size(),
        level.snapshot().cells().to_vec(),
        vec![replacement],
        Vec::new(),
    )
    .unwrap();
    let output = level.export(&changed).unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let encoded = value["entitites"][0][1]["stc"]["c"]["data"]
        .as_str()
        .unwrap();
    let patched = DataCodec::decode_gzip(encoded, wire.len()).unwrap().data;
    assert_eq!(patched, vec![0x5c, 0, 0x2b, 0, 0, 0x3c, 0, 0x4d]);
    assert_eq!(LegacyLevel::parse(&output).unwrap().snapshot(), &changed);
}

#[test]
fn block_transforms_and_delete_remain_legacy_exportable() {
    let board = token(&[0xff, 0xff]);
    let shape = token(&[0x07]);
    let json = format!(
        r#"{{"dur":0,"bes":[4,4],"bdat":"{board}","entitites":[["block",{{"eid":"b","g":{{"r":[0,0,2,2],"d":"{shape}"}},"cc":[[1,null,-1]]}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let mut timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    for (id, command) in [
        (
            "move-block",
            LevelCommand::MoveEntity {
                entity_id: EntityId::from("b"),
                origin: GridPoint::new(1, 1),
            },
        ),
        (
            "rotate-block",
            LevelCommand::RotateEntityClockwise {
                entity_id: EntityId::from("b"),
            },
        ),
        (
            "flip-block",
            LevelCommand::FlipEntityHorizontal {
                entity_id: EntityId::from("b"),
            },
        ),
    ] {
        timeline
            .apply(CommandEnvelope::new(
                CommandMetadata::new(id, "web", 100),
                command,
            ))
            .unwrap();
    }

    let transformed_json = level.export(timeline.snapshot()).unwrap();
    assert_eq!(
        LegacyLevel::parse(&transformed_json).unwrap().snapshot(),
        timeline.snapshot()
    );

    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("delete-block", "web", 101),
            LevelCommand::DeleteEntity {
                entity_id: EntityId::from("b"),
            },
        ))
        .unwrap();
    let deleted_json = level.export(timeline.snapshot()).unwrap();
    assert!(
        LegacyLevel::parse(&deleted_json)
            .unwrap()
            .snapshot()
            .entities()
            .is_empty()
    );
}

#[test]
fn blind_transforms_and_delete_without_sbln_remain_legacy_exportable() {
    let board = token(&[0xff, 0x0f]);
    let shape = token(&[0x03]);
    let wire = [0x1a, 0, 0x2b, 0, 0, 0x3c, 0, 0x4d];
    let canvas = gzip_token(&wire);
    let json = format!(
        r#"{{"dur":0,"bes":[4,3],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,2,1],"d":"{shape}"}},"stc":{{"c":{{"r":[4,2],"cmp":true,"data":"{canvas}"}}}}}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let mut timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    for (id, command) in [
        (
            "rotate-blind",
            LevelCommand::RotateEntityClockwise {
                entity_id: EntityId::from("p"),
            },
        ),
        (
            "move-blind",
            LevelCommand::MoveEntity {
                entity_id: EntityId::from("p"),
                origin: GridPoint::new(2, 0),
            },
        ),
        (
            "flip-blind",
            LevelCommand::FlipEntityHorizontal {
                entity_id: EntityId::from("p"),
            },
        ),
    ] {
        timeline
            .apply(CommandEnvelope::new(
                CommandMetadata::new(id, "web", 100),
                command,
            ))
            .unwrap();
    }

    let transformed_json = level.export(timeline.snapshot()).unwrap();
    assert_eq!(
        LegacyLevel::parse(&transformed_json).unwrap().snapshot(),
        timeline.snapshot()
    );

    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("delete-blind", "web", 101),
            LevelCommand::DeleteEntity {
                entity_id: EntityId::from("p"),
            },
        ))
        .unwrap();
    let deleted_json = level.export(timeline.snapshot()).unwrap();
    assert!(
        LegacyLevel::parse(&deleted_json)
            .unwrap()
            .snapshot()
            .entities()
            .is_empty()
    );
}

#[test]
fn sbcl_v2_locks_follow_collect_rows_and_default_rows_are_explicit() {
    let board = token(&[1]);
    let mut salt = SALT.to_vec();
    salt.extend_from_slice(b"SBCL");
    salt.extend_from_slice(&[2, 2, 0, 0b10]);
    let shape = DataCodec::encode_with_salt(&[1], &salt, false).unwrap();
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["block",{{"eid":"b","g":{{"r":[0,0,1,1],"d":"{shape}"}},"cc":[null,[7,4,12]]}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let PlaceableEntityKind::Block(block) = level.snapshot().entities()[0].kind() else {
        panic!("expected Block");
    };
    assert_eq!(
        block.collect_layers()[0].color_index(),
        DISABLED_COLLECT_COLOR_INDEX
    );
    assert!(!block.collect_layers()[0].is_locked());
    assert!(block.collect_layers()[1].is_locked());
    assert_eq!(
        block.collect_layers()[1].capacity(),
        CollectCapacity::Finite(12)
    );
    assert_eq!(level.export(level.snapshot()).unwrap(), json);
}

#[test]
fn decorators_resolve_after_entities_and_preserve_roles_and_count() {
    let board = token(&[0x07]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[3,1],"bdat":"{board}","entitites":[["ice",{{"eid":"i","deco":"key","count":7}}],["direction",{{"eid":"d","deco":"key","dir":"Vertical"}}],["key-locker",{{"eid":"k","deco":"key","lock":"locker"}}],["block",{{"eid":"key","g":{{"r":[0,0,1,1],"d":"{shape}"}},"cc":[]}}],["block",{{"eid":"locker","g":{{"r":[2,0,1,1],"d":"{shape}"}},"cc":[]}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    assert_eq!(
        level
            .entity_order()
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        ["i", "d", "k", "key", "locker"]
    );
    assert!(matches!(
        level.snapshot().decorators()[0].kind(),
        DecoratorKind::Direction {
            direction: CardinalDirection::Up,
            ..
        }
    ));
    assert!(matches!(
        level.snapshot().decorators()[1].kind(),
        DecoratorKind::Ice {
            blocking_count: 7,
            ..
        }
    ));
    assert!(matches!(
        level.snapshot().decorators()[2].kind(),
        DecoratorKind::KeyLocker { entity, key }
            if entity.as_str() == "locker" && key.as_str() == "key"
    ));
}

#[test]
fn glass_resolves_after_pool_and_patches_its_known_payload() {
    let board = token(&[1]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["glass",{{"eid":"g","deco":"p","count":2,"future":"keep"}}],["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":null}}}}}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    assert_eq!(level.glass_count(&DecoratorId::from("g")), Some(2));
    assert!(matches!(
        level.snapshot().decorators()[0].kind(),
        DecoratorKind::Glass {
            blocking_count: 2,
            ..
        }
    ));
    assert_eq!(level.export(level.snapshot()).unwrap(), json);

    let mut timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("set-glass", "web", 100),
            LevelCommand::SetGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
                blocking_count: 5,
            },
        ))
        .unwrap();
    let output = level.export(timeline.snapshot()).unwrap();
    assert!(output.contains(r#""count":5"#));
    assert!(output.contains(r#""future":"keep""#));
    assert_eq!(
        LegacyLevel::parse(&output).unwrap().snapshot(),
        timeline.snapshot()
    );

    let patched_level = LegacyLevel::parse(&output).unwrap();
    let mut disabled_timeline = LevelTimeline::new(patched_level.snapshot().clone()).unwrap();
    disabled_timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("disable-glass", "web", 101),
            LevelCommand::ToggleGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
            },
        ))
        .unwrap();
    let archived = patched_level.export(disabled_timeline.snapshot()).unwrap();
    assert!(archived.contains(r#""future":"keep""#));
    let archived_level = LegacyLevel::parse(&archived).unwrap();
    let mut enabled_timeline = LevelTimeline::new(archived_level.snapshot().clone()).unwrap();
    enabled_timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("enable-glass", "web", 102),
            LevelCommand::ToggleGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
            },
        ))
        .unwrap();
    assert!(
        archived_level
            .export(enabled_timeline.snapshot())
            .unwrap()
            .contains(r#""future":"keep""#)
    );
}

#[test]
fn new_active_glass_exports_with_the_legacy_marker() {
    let board = token(&[1]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":null}}}}}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let mut timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("set-glass", "web", 100),
            LevelCommand::SetGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
                blocking_count: 2,
            },
        ))
        .unwrap();
    let output = level.export(timeline.snapshot()).unwrap();
    assert!(output.contains(r#"["glass",{"eid":"g","deco":"p","count":2}]"#));
    assert_eq!(
        LegacyLevel::parse(&output).unwrap().snapshot(),
        timeline.snapshot()
    );
}

#[test]
fn glass_tombstones_move_between_active_entities_and_led_dt() {
    let board = token(&[1]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":null}}}}}}]],"_led":{{"future":{{"keep":[1,  2]}},"dt":[["custom-tomb",{{"opaque":[1,  2]}}]]}}}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let mut active_timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    active_timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("set-glass", "web", 100),
            LevelCommand::SetGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
                blocking_count: 2,
            },
        ))
        .unwrap();
    let active = level.export(active_timeline.snapshot()).unwrap();
    let active_level = LegacyLevel::parse(&active).unwrap();

    let mut disabled_timeline = LevelTimeline::new(active_level.snapshot().clone()).unwrap();
    disabled_timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("disable-glass", "web", 101),
            LevelCommand::ToggleGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
            },
        ))
        .unwrap();
    let disabled = active_level.export(disabled_timeline.snapshot()).unwrap();
    let disabled_value: serde_json::Value = serde_json::from_str(&disabled).unwrap();
    assert_eq!(disabled_value["entitites"].as_array().unwrap().len(), 1);
    let tombstones = disabled_value["_led"]["dt"].as_array().unwrap();
    assert!(
        tombstones.iter().any(|entry| {
            entry[0] == "glass" && entry[1]["eid"] == "g" && entry[1]["count"] == 0
        })
    );
    assert!(tombstones.iter().any(|entry| {
        entry[0] == "custom-tomb" && entry[1]["opaque"] == serde_json::json!([1, 2])
    }));
    assert!(disabled.contains(r#""future":{"keep":[1,  2]}"#));

    let disabled_level = LegacyLevel::parse(&disabled).unwrap();
    assert_eq!(
        disabled_level
            .snapshot()
            .glass_blocking_count(&EntityId::from("p")),
        0
    );
    assert_eq!(disabled_level.glass_count(&DecoratorId::from("g")), Some(0));
    let mut reenabled_timeline = LevelTimeline::new(disabled_level.snapshot().clone()).unwrap();
    reenabled_timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("enable-glass", "web", 102),
            LevelCommand::ToggleGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
            },
        ))
        .unwrap();
    let reenabled = disabled_level
        .export(reenabled_timeline.snapshot())
        .unwrap();
    let reenabled_value: serde_json::Value = serde_json::from_str(&reenabled).unwrap();
    assert!(
        reenabled_value["entitites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry[0] == "glass" && entry[1]["eid"] == "g" && entry[1]["count"] == 1
            })
    );
    assert!(
        !reenabled_value["_led"]["dt"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry[0] == "glass" })
    );
    assert_eq!(
        LegacyLevel::parse(&reenabled).unwrap().snapshot(),
        reenabled_timeline.snapshot()
    );
}

#[test]
fn new_zero_glass_creates_led_dt() {
    let board = token(&[1]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":null}}}}}}]]}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let mut timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("set-glass", "web", 100),
            LevelCommand::SetGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
                blocking_count: 0,
            },
        ))
        .unwrap();
    let output = level.export(timeline.snapshot()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(
        value["_led"]["dt"].as_array().unwrap().iter().any(|entry| {
            entry[0] == "glass" && entry[1]["eid"] == "g" && entry[1]["count"] == 0
        })
    );
    assert_eq!(
        LegacyLevel::parse(&output).unwrap().snapshot(),
        timeline.snapshot()
    );
}

#[test]
fn active_zero_glass_is_rejected_in_favor_of_led_dt() {
    let board = token(&[1]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["glass",{{"eid":"g","deco":"p","count":0}}],["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":null}}}}}}]]}}"#
    );
    assert!(matches!(
        LegacyLevel::parse(&json),
        Err(LegacyError::InvalidField { message, .. }) if message.contains("_led.dt")
    ));
}

#[test]
fn opaque_led_dt_is_preserved_and_blocks_new_glass_tombstones() {
    let board = token(&[1]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":null}}}}}}]],"_led":{{"dt":"opaque"}}}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    assert_eq!(level.export(level.snapshot()).unwrap(), json);

    let mut timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("set-glass", "web", 100),
            LevelCommand::SetGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
                blocking_count: 0,
            },
        ))
        .unwrap();
    assert!(matches!(
        level.export(timeline.snapshot()),
        Err(LegacyError::UnsupportedChange { entity_id, .. }) if entity_id == "_led"
    ));
}

#[test]
fn opaque_led_dt_ids_block_conflicting_new_decorators() {
    let board = token(&[1]);
    let shape = token(&[1]);
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":null}}}}}}]],"_led":{{"dt":[["future",{{"eid":"g"}}]]}}}}"#
    );
    let level = LegacyLevel::parse(&json).unwrap();
    let mut timeline = LevelTimeline::new(level.snapshot().clone()).unwrap();
    timeline
        .apply(CommandEnvelope::new(
            CommandMetadata::new("set-glass", "web", 100),
            LevelCommand::SetGlass {
                decorator_id: DecoratorId::from("g"),
                entity_id: EntityId::from("p"),
                blocking_count: 1,
            },
        ))
        .unwrap();
    assert!(matches!(
        level.export(timeline.snapshot()),
        Err(LegacyError::DuplicateEntityId(id)) if id == "g"
    ));
}

struct CustomCodec;

impl UnknownEntityCodec for CustomCodec {
    fn marker(&self) -> &str {
        "custom"
    }

    fn validate(&self, payload: &serde_json::Value) -> Result<(), String> {
        payload
            .get("opaque")
            .is_some()
            .then_some(())
            .ok_or_else(|| "opaque is required".to_owned())
    }
}

#[test]
fn unknown_markers_are_lossless_but_read_only_without_a_plugin_codec() {
    let json = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{}","entitites":[["custom",{{"eid":"x","opaque":[1,  2]}}]]}}"#,
        token(&[1])
    );
    let level = LegacyLevel::parse(&json).unwrap();
    assert!(matches!(
        level.compatibility_state(),
        CompatibilityState::ReadOnly { unknown_markers } if unknown_markers == &["custom"]
    ));
    assert_eq!(level.export(level.snapshot()).unwrap(), json);
    assert!(matches!(
        level.export_with_duration(level.snapshot(), 1.0),
        Err(LegacyError::ReadOnlyUnknownMarkers { .. })
    ));

    let writable = LegacyLevel::parse_with_codecs(&json, &[&CustomCodec]).unwrap();
    assert_eq!(
        writable.compatibility_state(),
        &CompatibilityState::Writable
    );
    let output = writable
        .export_with_duration(writable.snapshot(), 1.0)
        .unwrap();
    assert!(output.contains(r#"["custom",{"eid":"x","opaque":[1,  2]}]"#));
}

#[test]
fn malformed_codec_canvas_and_references_are_typed_errors() {
    assert!(matches!(
        DataCodec::decode("%%%:AA==:00000000"),
        Err(CodecError::Base64 { segment: "salt" })
    ));
    assert!(matches!(
        DataCodec::decode("AAECAwQFBgc=:BQ==:00000000"),
        Err(CodecError::Checksum { .. })
    ));

    let bad_board = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{}","entitites":[]}}"#,
        "AAECAwQFBgc=:BQ==:00000000"
    );
    assert!(matches!(
        LegacyLevel::parse(&bad_board),
        Err(LegacyError::DataCodec {
            source: CodecError::Checksum { .. },
            ..
        })
    ));

    let board = token(&[0x03]);
    let shape = token(&[1]);
    let not_gzip = token(&[0x1c]);
    let bad_canvas = format!(
        r#"{{"dur":0,"bes":[2,1],"bdat":"{board}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":"{not_gzip}"}}}}}}]]}}"#
    );
    assert!(matches!(
        LegacyLevel::parse(&bad_canvas),
        Err(LegacyError::DataCodec {
            source: CodecError::Gzip,
            ..
        })
    ));

    let missing_reference = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{}","entitites":[["ice",{{"eid":"i","deco":"missing","count":1}}]]}}"#,
        token(&[1])
    );
    assert!(matches!(
        LegacyLevel::parse(&missing_reference),
        Err(LegacyError::InvalidReference { .. })
    ));

    let duplicate_reference = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{}","entitites":[["block",{{"eid":"b","g":{{"r":[0,0,1,1],"d":"{shape}"}},"cc":[]}}],["ice",{{"eid":"i1","deco":"b","count":1}}],["ice",{{"eid":"i2","deco":"b","count":2}}]]}}"#,
        token(&[1])
    );
    assert!(matches!(
        LegacyLevel::parse(&duplicate_reference),
        Err(LegacyError::InvalidField { message, .. }) if message.contains("more than one Ice")
    ));
}

#[test]
fn duplicate_ids_and_malformed_sbln_framing_are_rejected() {
    let board = token(&[0x03]);
    let shape = token(&[1]);
    let duplicate = format!(
        r#"{{"dur":0,"bes":[2,1],"bdat":"{board}","entitites":[["block",{{"eid":"same","g":{{"r":[0,0,1,1],"d":"{shape}"}},"cc":[]}}],["block",{{"eid":"same","g":{{"r":[1,0,1,1],"d":"{shape}"}},"cc":[]}}]]}}"#
    );
    assert_eq!(
        LegacyLevel::parse(&duplicate).unwrap_err(),
        LegacyError::DuplicateEntityId("same".to_owned())
    );

    let mut salt = SALT.to_vec();
    salt.extend_from_slice(b"SBLN");
    salt.extend_from_slice(&[1, 1, 0, 1, 0]);
    let malformed_shape = DataCodec::encode_with_salt(&[1], &salt, false).unwrap();
    let canvas = gzip_token(&[0x1c]);
    let malformed = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{malformed_shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":"{canvas}"}}}}}}]]}}"#,
        token(&[1])
    );
    assert!(matches!(
        LegacyLevel::parse(&malformed),
        Err(LegacyError::InvalidField { message, .. }) if message.contains("SBLN length")
    ));

    salt.push(0);
    let preserved_shape = DataCodec::encode_with_salt(&[1], &salt, false).unwrap();
    let preserved = format!(
        r#"{{"dur":0,"bes":[1,1],"bdat":"{}","entitites":[["pool",{{"eid":"p","g":{{"r":[0,0,1,1],"d":"{preserved_shape}"}},"stc":{{"c":{{"r":[1,1],"cmp":true,"data":"{canvas}"}}}}}}]]}}"#,
        token(&[1])
    );
    let level = LegacyLevel::parse(&preserved).unwrap();
    assert_eq!(level.export(level.snapshot()).unwrap(), preserved);
    let changed_blind = Blind::new(
        2,
        vec![BlindTile::from_colors(2, vec![1, 0, 0, 0]).unwrap()],
    )
    .unwrap();
    let replacement = PlaceableEntity::blind(
        "p",
        GridPoint::new(0, 0),
        level.snapshot().entities()[0].shape(),
        changed_blind,
    )
    .unwrap();
    let changed = LevelSnapshot::from_parts(
        level.snapshot().size(),
        level.snapshot().cells().to_vec(),
        vec![replacement],
        Vec::new(),
    )
    .unwrap();
    assert!(matches!(
        level.export(&changed),
        Err(LegacyError::UnsupportedMetadataChange {
            metadata: "SBLN",
            ..
        })
    ));
}
