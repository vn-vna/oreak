use std::collections::BTreeSet;

use oreak_plugin_api::{
    Capability, CellOffset, CodecCertificationState, CollisionSemantics, ContentHash,
    CustomEntityContribution, CustomEntityInstance, HookDeclaration, HookKind,
    LegacyCodecCertification, LegacyCodecDeclaration, LocalId, PluginId, PluginManifest,
    ReleaseGateIssueKind, SchemaContribution, SelectionSemantics, SpatialEnvelope, Validate,
    ValidatorContribution, ValidatorScope, Version, evaluate_release_gate,
};
use serde_json::json;

fn id(value: &str) -> LocalId {
    LocalId::new(value).unwrap()
}

fn manifest() -> PluginManifest {
    let mut manifest = PluginManifest::new(
        PluginId::new("com.example.cells").unwrap(),
        Version::new(1, 2, 3),
        Version::new(1, 0, 0),
        ContentHash::blake3(b"plugin source"),
    );
    manifest.capabilities = BTreeSet::from([
        Capability::ContributeSchema,
        Capability::ContributeCustomEntities,
        Capability::Validate,
        Capability::LegacyLevelDataCodec,
    ]);
    manifest.hooks = vec![
        HookDeclaration {
            id: id("validate_level"),
            kind: HookKind::Validate,
            export: id("validate_level"),
        },
        HookDeclaration {
            id: id("encode_legacy"),
            kind: HookKind::EncodeLegacyLevelData,
            export: id("encode_legacy"),
        },
        HookDeclaration {
            id: id("decode_legacy"),
            kind: HookKind::DecodeLegacyLevelData,
            export: id("decode_legacy"),
        },
    ];
    manifest.schemas.push(SchemaContribution {
        id: id("cell_data"),
        version: 1,
        title: "Cell data".to_owned(),
        root_schema: json!({"type": "object"}),
    });
    manifest.custom_entities.push(CustomEntityContribution {
        id: id("custom_cell"),
        display_name: "Custom cell".to_owned(),
        data_schema: id("cell_data"),
        data_schema_version: 1,
        runtime: true,
    });
    manifest.validators.push(ValidatorContribution {
        id: id("level_validator"),
        hook: id("validate_level"),
        scope: ValidatorScope::Level,
    });
    manifest.legacy_level_data_codec = LegacyCodecDeclaration::Provided {
        codec_version: Version::new(2, 0, 0),
        encode_hook: id("encode_legacy"),
        decode_hook: id("decode_legacy"),
        fixture_set_hash: ContentHash::blake3(b"fixtures"),
    };
    manifest
}

fn runtime_entity(manifest: &PluginManifest) -> CustomEntityInstance {
    CustomEntityInstance {
        entity_id: "entity-1".to_owned(),
        plugin: manifest.exact_pin(),
        entity_type: id("custom_cell"),
        runtime: true,
        envelope: SpatialEnvelope {
            origin: oreak_plugin_api::GridPoint { x: 4, y: 7 },
            occupied_cells: vec![CellOffset { x: 0, y: 0 }, CellOffset { x: 1, y: 0 }],
            collision: CollisionSemantics::Solid {
                layer: id("placeables"),
                collides_with: BTreeSet::from([id("placeables")]),
            },
            selection: SelectionSemantics::OccupiedCells,
        },
        plugin_data: json!({"strength": 3}),
    }
}

#[test]
fn manifest_round_trips_and_validates() {
    let manifest = manifest();
    manifest.validate().unwrap();

    let json = serde_json::to_string(&manifest).unwrap();
    let decoded: PluginManifest = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, manifest);
}

#[test]
fn serde_rejects_invalid_identifiers_hashes_and_unknown_fields() {
    let invalid_id = r#"{
        "manifest_version": 1,
        "plugin_id": "Bad Plugin",
        "plugin_version": "1.0.0",
        "api_version": "1.0.0",
        "source_hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }"#;
    assert!(serde_json::from_str::<PluginManifest>(invalid_id).is_err());

    let invalid_hash = invalid_id
        .replace("Bad Plugin", "com.example.plugin")
        .replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "ABC",
        );
    assert!(serde_json::from_str::<PluginManifest>(&invalid_hash).is_err());

    let unknown_capability = invalid_id
        .replace("Bad Plugin", "com.example.plugin")
        .replace(
            "\n    }",
            ",\n        \"capabilities\": [\"filesystem\"]\n    }",
        );
    assert!(serde_json::from_str::<PluginManifest>(&unknown_capability).is_err());

    let mut value = serde_json::to_value(manifest()).unwrap();
    value["filesystem"] = json!(true);
    assert!(serde_json::from_value::<PluginManifest>(value).is_err());
}

#[test]
fn validation_rejects_missing_capability_and_duplicate_cells() {
    let mut manifest = manifest();
    manifest.capabilities.remove(&Capability::Validate);
    assert!(manifest.validate().is_err());

    let mut entity = runtime_entity(&manifest);
    entity
        .envelope
        .occupied_cells
        .push(CellOffset { x: 0, y: 0 });
    let errors = entity.validate().unwrap_err();
    assert!(
        errors
            .issues
            .iter()
            .any(|issue| issue.code == "duplicate_occupied_cell")
    );
}

#[test]
fn release_gate_requires_certification_for_the_exact_pin_and_codec_version() {
    let manifest = manifest();
    let entity = runtime_entity(&manifest);
    let pin = manifest.exact_pin();

    let missing = evaluate_release_gate(
        std::slice::from_ref(&pin),
        std::slice::from_ref(&manifest),
        std::slice::from_ref(&entity),
        &[],
    );
    assert!(!missing.allowed);
    assert_eq!(
        missing.issues[0].kind,
        ReleaseGateIssueKind::CertificationMissing
    );

    let wrong_version = LegacyCodecCertification {
        plugin: pin.clone(),
        state: CodecCertificationState::Certified {
            codec_version: Version::new(1, 0, 0),
            certificate_hash: ContentHash::blake3(b"certificate"),
        },
    };
    let mismatch = evaluate_release_gate(
        std::slice::from_ref(&pin),
        std::slice::from_ref(&manifest),
        std::slice::from_ref(&entity),
        &[wrong_version],
    );
    assert_eq!(
        mismatch.issues[0].kind,
        ReleaseGateIssueKind::CertifiedCodecVersionMismatch
    );

    let mut other_pin = pin.clone();
    other_pin.plugin_version = Version::new(1, 2, 4);
    let other_version_certificate = LegacyCodecCertification {
        plugin: other_pin,
        state: CodecCertificationState::Certified {
            codec_version: Version::new(2, 0, 0),
            certificate_hash: ContentHash::blake3(b"other certificate"),
        },
    };
    let wrong_plugin_version = evaluate_release_gate(
        std::slice::from_ref(&pin),
        std::slice::from_ref(&manifest),
        std::slice::from_ref(&entity),
        &[other_version_certificate],
    );
    assert_eq!(
        wrong_plugin_version.issues[0].kind,
        ReleaseGateIssueKind::CertificationMissing
    );

    let certified = LegacyCodecCertification {
        plugin: pin.clone(),
        state: CodecCertificationState::Certified {
            codec_version: Version::new(2, 0, 0),
            certificate_hash: ContentHash::blake3(b"certificate"),
        },
    };
    assert!(evaluate_release_gate(&[pin], &[manifest], &[entity], &[certified]).is_allowed());
}

#[test]
fn editor_only_custom_entities_do_not_require_a_runtime_codec() {
    let mut manifest = manifest();
    manifest.legacy_level_data_codec = LegacyCodecDeclaration::NotProvided;
    manifest
        .capabilities
        .remove(&Capability::LegacyLevelDataCodec);
    let mut entity = runtime_entity(&manifest);
    entity.runtime = false;

    assert!(
        evaluate_release_gate(
            &[manifest.exact_pin()],
            std::slice::from_ref(&manifest),
            &[entity],
            &[]
        )
        .is_allowed()
    );
}
