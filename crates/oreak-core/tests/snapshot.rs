use oreak_core::LevelSnapshot;

#[test]
fn snapshot_deserialization_rejects_unknown_schema_versions() {
    let error = serde_json::from_str::<LevelSnapshot>(
        r#"{
            "schema_version": 2,
            "size": { "width": 2, "height": 2 },
            "cells": ["Floor", "Floor", "Floor", "Floor"]
        }"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unsupported level schema version 2")
    );
}

#[test]
fn snapshot_deserialization_rejects_mismatched_cell_counts() {
    let error = serde_json::from_str::<LevelSnapshot>(
        r#"{
            "schema_version": 1,
            "size": { "width": 2, "height": 2 },
            "cells": ["Floor"]
        }"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("1 cells but 4 were expected"));
}

#[test]
fn valid_snapshot_round_trips_through_json() {
    let snapshot = LevelSnapshot::new(8, 8).unwrap();
    let json = serde_json::to_string(&snapshot).unwrap();
    let restored: LevelSnapshot = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, snapshot);
    assert_eq!(restored.content_hash(), snapshot.content_hash());
}
