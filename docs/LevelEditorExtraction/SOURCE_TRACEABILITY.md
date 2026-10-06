# Source Traceability and Coverage

This appendix records where the extracted behavior came from. It is intentionally implementation-oriented; the other documents are the implementation-independent specification.

## 1. Authoritative decisions and known documentation drift

Current executable data contracts and tests take precedence over the old in-folder README.

| Topic | Authoritative evidence | Extraction decision |
|---|---|---|
| Pipe format | `LevelEditorLevelJsonAdapter.cs` constants/read/write; `LevelEditorPipeAuthoring.cs`; runtime `PipeDecoratorData.cs` | Current payload is `eid/deco/m/fr/s`; multiple Pipes per Blind; enabled mouths cannot overlap. |
| Old Pipe description | `README.md` “one Pipe” and `edge/start/width/layers/unit` section | Obsolete; excluded from canonical/wire schema. |
| Collect row | Adapter read/write and runtime collect-config decoder | Four semantic values, including maximum speed. |
| Radius default | Runtime collect config and project-default resolver | Null resolves project defaults; runtime fallback 20. Old README value 30 is stale. |
| Map invalid release | `LevelEditorMapResizeGesture.cs` vs move/Blind resize tests | Replacement specification chooses consistent baseline rollback instead of last-valid commit. |
| Divide parts | semantic options vs UI state | Domain supports 2..16; current UI exposes up to 8. Server supports 16. |
| Sandbox parity | simulation code and Pipe properties | Deterministic authoring subset; no Pipe, Glass, Hammer/Wall, or max-speed parity. |

## 2. Primary model and persistence evidence

- Canonical immutable document, history, commands, transforms, merge, resize, validation: `LevelEditorDocument.cs`.
- Shapes, Placeables, collect-layer schema, selection, drag payloads: `LevelEditorShapesAndPlaceables.cs`.
- Decorator kinds, counted/directional/linked/Pipe fields and positions: `LevelEditorDecorators.cs`.
- Pixel canvas, guide topology, Brush partitions/operations: `LevelEditorBrushModel.cs`.
- Legacy root/entity parsing, two-pass resolution, DataCodec metadata, preservation-oriented export: `LevelEditorLevelJsonAdapter.cs`.
- Boundary resolution/loss analysis: `LevelEditorPoolBoundary.cs`.
- Current Pipe geometry validation and conversion: `LevelEditorPipeAuthoring.cs` and runtime `PipeDecoratorData.cs` / `PipeMouthGeometry.cs`.

## 3. Complete LevelEditor file coverage map

All files are under `Assets/__Project/Scripts/Editor/LevelEditor/`.

### Aggregate, window, navigation, layout

- `README.md` — legacy product narrative; cross-checked and corrected where stale.
- `SandLevelEditorWindow.cs` — command orchestration, properties, shortcuts, modes, save/open/recovery, history, proposals, PLAY.
- `SandLevelEditorSceneOverlay.cs` — scene-overlay entry point.
- `LevelEditorDocument.cs` — canonical aggregate, immutable snapshots, history, mutation validation.
- `LevelEditorModeNavigator.cs` — five modes, hints, layout, Real Test entry.
- `LevelEditorModeNavigatorSelfTests.cs` — mode descriptor/geometry/transition expectations.
- `LevelEditorWorkspaceLayout.cs` — three-pane sizing and splitter behavior.
- `LevelEditorGridViewport.cs` — projection, hit testing, input ownership, selection, rendering order, integration.
- `LevelEditorDurationPropertiesPane.cs` — delayed timing input and presets.
- `LevelEditorDurationSelfTests.cs` — duration defaults/history/JSON/recovery/PLAY.

### Shape, placeable, selection, map, visuals

- `LevelEditorShapesAndPlaceables.cs` — shape/placeable/collect-layer/selection models.
- `LevelEditorShapeDesignerPane.cs` — Shape authoring and collection UX.
- `LevelEditorShapeSampleLibrary.cs` — reusable Shape library persistence/migration.
- `LevelEditorPlaceableMoveGesture.cs` — move/duplicate/rotate/recolor candidate gesture.
- `LevelEditorBlindResizeGesture.cs` — Blind rail resize gesture.
- `LevelEditorMapResizeGesture.cs` — board frame resize gesture.
- `LevelEditorWallStrokeGesture.cs` — Floor/Wall strokes.
- `LevelEditorGridEdit.cs` — direct placeable-footprint editing.
- `LevelEditorGridEditSelfTests.cs` — extension/removal/connectivity/paint protection.
- `LevelEditorSweepSelectionSelfTests.cs` — marquee selection normalization/intersection.
- `LevelEditorPlaceableVisuals.cs` — Block layer rims and Blind container visuals.

### Brush, guides, Divide, coordinate/rendering

- `LevelEditorBrushModel.cs` — canvas, topology, partition, operations.
- `LevelEditorBrushToolState.cs` — tool settings/preferences and UI constraints.
- `LevelEditorBrushPropertiesPane.cs` — Paint/Fill/Line/Divide/CAPX properties.
- `LevelEditorBrushGesture.cs` — Paint/Erase/Fill transactions.
- `LevelEditorBrushLineGesture.cs` — guide Line snapping/rasterization.
- `LevelEditorBrushLineAssistRenderer.cs` — Line snapping/partition visual assistance.
- `LevelEditorBrushDivide.cs` — options, session, deterministic planner.
- `LevelEditorGuidePartitionAnalyzer.cs` — guide-defined group analysis.
- `LevelEditorBrushRecoveryCodec.cs` — strict recovery encoding/decoding.
- `LevelEditorBlindPixelRenderer.cs` — exact/mip artwork rendering.
- `LevelEditorBlindCoordinateRuler.cs` — target selection, tick/label/ruler geometry.

### Decorators, relations, Pipes, boundaries

- `LevelEditorDecorators.cs` — decorator canonical model.
- `LevelEditorDecoratorVisuals.cs` — Ice/Direction/Glass badge/rim visualization.
- `LevelEditorDecoratorPositionDragGesture.cs` — persistent badge positioning.
- `LevelEditorKeyLockerVisuals.cs` — linked endpoint badges/lines shared by relation visuals.
- `LevelEditorHammerWallSelfTests.cs` — Hammer/Wall relation, lifecycle, JSON, Sandbox isolation.
- `LevelEditorPipeAuthoring.cs` — current Pipe conversion/validation/non-overlap.
- `LevelEditorPipeProperties.cs` — current multiple-Pipe properties and stack model.
- `LevelEditorPipeMouthGesture.cs` — mouth create/resize/slide gestures.
- `LevelEditorPipeSelfTests.cs` — Pipe constraints/history/JSON/transform/invalid-host behavior.
- `LevelEditorPoolBoundary.cs` — inherited/authored geometry, loss reports, batch edits.
- `LevelEditorPoolBoundarySelfTests.cs` — packing/loss/serialization/history/runtime policy.

### Capacity defaults, palette, distribution, picture-grid migration

- `LevelEditorBlockCollectDefaults.cs` — project-default resolution and change invalidation.
- `LevelEditorColorProfilePalette.cs` — stable color-slot lookup and diagnosed fallback colors.
- `LevelEditorQuickColorDistribution.cs` — region plans, weights, deterministic distribution UI/model.
- `LevelEditorQuickColorDistributionSelfTests.cs` — weighting, locks, effective sand, group selection.
- `LevelEditorPictureGridMigration.cs` — artwork resample plus capacity scaling.
- `LevelEditorPictureGridWindow.cs` — current/batch preview and apply workflow.
- `LevelEditorPictureGridCatalogBridge.cs` — catalog snapshot/fingerprint/errors abstraction.
- `LevelEditorPictureGridBatch.cs` — eligibility, budgets, backup/write/rollback transaction.
- `LevelEditorPictureGridSelfTests.cs` — scaling/remainders/layers/locks/loss/limits.
- `LevelEditorPictureGridJsonSelfTests.cs` — resolution metadata and JSON compatibility.
- `LevelEditorPictureGridBatchSelfTests.cs` — stale/skip/dedup/rollback/catalog/budget cases.
- `LevelEditorPictureGridWindowSelfTests.cs` — window/current-level and catalog listing cases.

### CAPX

- `LevelEditorCapxSourceLoader.cs` — project/external source loading and limits.
- `LevelEditorCapxImage.cs` — immutable source, color-space conversion, deterministic mapping.
- `LevelEditorCapxColorReviewWindow.cs` — explicit mapping review/acceptance.
- `LevelEditorCapxImportPane.cs` — staged source/review/preview/apply state.
- `LevelEditorCapxPlacement.cs` — fit modes, sampling, partition writes, metrics, stale checks.
- `LevelEditorCapxPlacementGesture.cs` — move/resize placement gesture.
- `LevelEditorCapxImageSelfTests.cs` — source/CIEDE2000/mapping/alpha behavior.
- `LevelEditorCapxImportSelfTests.cs` — source adapter/history/persistence/gesture geometry.
- `LevelEditorCapxPlacementSelfTests.cs` — sampling/fit/write/staleness.
- `LevelEditorCapxUiSelfTests.cs` — native interaction regression coverage.

### Sand Studio

- `LevelEditorSandStudio.cs` — private canvas tools, Apply, rendering.
- `LevelEditorSandStudioImmersive.cs` — pan/zoom/custom brush/selection/local history.
- `LevelEditorSandStudioSelfTests.cs` — brush/mask/fill/pick/custom/selection/gradient.

### Sandbox and test launches

- `LevelEditorSandboxPlay.cs` — private session, move/delete/history/decorator gates.
- `LevelEditorSandboxAnalogDragGesture.cs` — analog movement and path preview.
- `LevelEditorSandboxPlayableGeometry.cs` — transient effective boundary projection.
- `LevelEditorSandboxSandSimulation.cs` — deterministic gravity/contact/collection/counters.
- `LevelEditorSandboxSandSimulationSelfTests.cs` — detailed tick, lane, layer, capacity, history contracts.
- `LevelEditorSandboxLockerSelfTests.cs` — Key/Locker movement and release timing.
- `LevelEditorGameplayPlayBridge.cs` — authored-draft export and launch abstraction.
- `LevelEditorGameplayPlaySelfTests.cs` — footer/export/invalid launch behavior.
- `LevelEditorRealTestSession.cs` — dedicated runtime round trip and state restoration.
- `LevelEditorRealTestSelfTests.cs` — Exit/Unity Stop async round-trip assertions.

### Cross-cutting test suite

- `SandLevelEditorSelfTests.cs` — broad workspace, geometry, transform, merge, layer, decorator, Brush, JSON, recovery, shortcut coverage.

## 4. Direct runtime/plugin contracts inspected

These live outside the requested folder but define the wire behavior the editor must satisfy.

- `Assets/__Project/Scripts/Runtime/Levels/LevelData.cs` — root property names and entity envelopes.
- `Assets/__Project/Scripts/Runtime/Levels/LevelDataDefaults.cs` — project fallback values.
- `Assets/__Project/Scripts/Runtime/Entt/SandBlockCollectConfig.cs` — collect row runtime defaults/meaning.
- `Assets/__Project/Scripts/Runtime/Entt/SandBlockCollectConfigDecoder.cs` — four-element `cc` runtime decoder.
- `Assets/__Project/Scripts/Runtime/Entt/PipeDecoratorData.cs` — current `m/fr/s` Pipe contract.
- `Assets/__Project/Scripts/Runtime/Entt/PipeMouthGeometry.cs` — boundary, gravity, inlet, reserve constraints.
- `Assets/__Project/Scripts/Runtime/Entt/SandPoolCanvasUtility.cs` — authoritative Pool canvas layout, padding/corner classification, and playable pixels.
- `Assets/__Project/Scripts/Runtime/Entt/SandPoolSpawnData.cs` and `SandTexConfig.cs` — Pool spawn, boundary, gravity, and runtime physics fields.
- `Assets/__Project/Scripts/Runtime/Entt/IceDecoratorData.cs`, `GlassDecoratorData.cs`, and `DirectionDecoratorData.cs` — runtime single-host decorator DTOs/converters.
- `Assets/__Project/Scripts/Runtime/Entt/KeyLockerDecoratorData.cs`, `KeyDecoratorData.cs`, `LockDecoratorData.cs`, and `HammerWallDecoratorData.cs` — runtime linked-decorator compatibility contracts.
- `Assets/__Project/Scripts/Runtime/Entt/SandCanvas.cs` — runtime canvas conventions.
- `Assets/__Project/Scripts/Runtime/Utilities/SpecializedData.cs` — compact grid data.
- `Assets/__Project/Scripts/Runtime/Utilities/EntityDataDecoder.cs` — DataCodec implementation.
- `Assets/Plugins/Magima/Magima Puzzle/Pixel Canvas/PixelCanvas.cs` and `PixelCanvasSerializer.cs` — CAPX contract.

## 5. Test-derived edge cases retained in the specification

- zero/Unlimited/disabled/repeated-color collect rows;
- per-row locks and stale lock metadata;
- elongated Blind footprints;
- footprint holes in art, guides, Pipe mouths, collection, and CAPX fit;
- same-color sequential row promotion delayed until next tick;
- mismatch shielding behind first contact grain;
- stable competing-Block ownership independent of serialized order;
- automatic Sandbox history coalescing;
- current-draft PLAY from Sandbox;
- recovery baseline vs current duration/artwork;
- unknown JSON fields and raw token preservation;
- placeholder canvas resolution inference;
- active vs tombstoned decorators and endpoint position persistence;
- catalog aliases, skipped files, stale files, verified rollback;
- CAPX mapping tie order, alpha threshold, original-source resampling;
- stale proposals across Divide, CAPX, migration, normalization, and Sand Studio.

## 6. Coverage limitation

This documentation describes all substantive `.cs` files in the target folder and directly required runtime/plugin contracts. Unity `.meta` files and the assembly-definition metadata contain no mechanic/schema behavior and were intentionally excluded from the semantic specification.
