# Level Editor Extraction Gap & Implementation Plan

## Scope and terminology

This plan compares the extracted target contract in [`LevelEditorExtraction`](LevelEditorExtraction/README.md) with the current browser-first Rust editor. The extraction calls the private canvas workflow **Sand Studio**; this plan uses **Color/Sand Studio** for the requested Color Studio feature.

The plan deliberately preserves Oreak's existing strengths: deterministic core commands, append-only collaborative timeline, server-ordered events, and the browser's shared core model. It does **not** attempt to port Unity UI structure or make the browser a domain authority.

## Implemented increment: Glass decorator vertical slice

The first implementation increment adds **Glass on a Blind/Pool** without introducing a third placeable type:

- Core: `Glass { entity, blocking_count }` is Blind-only, unique per Blind, deterministic in hashes, participates in deletion cascade, and supports Ice-style stable-ID count-zero tombstones with typed `ToggleGlass` and `SetGlass` timeline commands.
- Browser: a selected Pool exposes Glass toggle/count controls, `G` shortcut support, command-history labels, and an active Glass canvas overlay. The client continues to submit typed commands to the authoritative server.
- Protocol/server: generic command serialization and the existing authoritative apply/broadcast path carry Glass; focused authenticated RPC coverage proves it.
- Legacy adapter: active `"glass"` envelopes (`eid`, `deco`, `count >= 1`) and count-zero Glass tombstones under `_led.dt` parse, preserve unknown payload fields, move safely between active/archive storage, and re-export. Opaque `_led` values or non-array `dt` data are never overwritten; a change that would require doing so fails explicitly.

**Next trigger:** generalize the archived-decorator sidecar and explicit enabled/settings model beyond Glass (starting with Ice), before claiming broad decorator tombstone parity. Do not silently serialize any disabled decorator as an active runtime envelope.

## Current assessment

| Area | Current capability | Target delta | Priority |
| --- | --- | --- | --- |
| Primary entity authoring | Blocks/Blinds, connected shapes, placement, shape samples, movement, rotation, flip, delete, and a basic shape designer already exist. | Preserve placed-order/source metadata; add complete entity properties and shared level settings. Merge and Blind topology resize remain absent. | P0 |
| Level aggregate | Grid, entities, decorators, and deterministic hashes live in `oreak-core`. | Add duration, palette, global pixels-per-cell, passthrough runtime metadata, compatibility envelope, and globally unique IDs. Preserve import order rather than sorting entities/decorators by ID. | P0 |
| Block collection layers | Color, radius, capacity, and lock are represented; the web surface is read-only. | Add disabled/inherited semantics, optional max collection speed, ordered layer editing commands/UI, project defaults, and capacity analysis. | P0 |
| Decorators | Ice, Direction, one-key/one-lock KeyLocker, and Blind-hosted Glass are validated and rendered; Ice and Glass preserve stable count-zero identities. | Generalize tombstones to every kind, add multi-key relations, Hammer/Wall, Pipe, badge positions, batch behavior, and complete legacy support. | P0/P1 |
| Color and Brush | Per-Blind paint/erase/fill and guide data exist; Brush colors currently stop at 10. | Model a project color profile C0..C15; support C1..C15 authoring, unavailable-slot diagnostics, richer brushes/guide authoring, and Color/Sand Studio. | P1 |
| Legacy adapter | Loss-preserving JSON, Block/Blind/Ice/Direction/KeyLocker, active and `_led.dt` Glass, DataCodec, and unknown-record safety exist. | Import/export new Level metadata, 4-value collect rows, Hammer/Pipe, generalized tombstones, icon metadata, shared pixels-per-cell, and current Pipe schema. | P1 |
| Collaboration/transactions | Commands are server-serialized and deduplicated by command ID. | Mutation requests lack a client-supplied expected sequence/revision; proposals and stale-safe Apply are missing. | P1 |
| Storage | In-memory timelines and inline snapshot artwork. | Content-addressed canvas blobs, durable revisions, external-source fingerprinting, recovery, and migrations are later platform work. | P2 |

### Important structural differences to resolve first

1. [`LevelSnapshot`](../crates/oreak-core/src/level.rs) sorts entities and decorators by ID. The extracted contract says import order is meaningful and must survive round trips. Replace sort-dependent lookup with a derived lookup/index while retaining ordered vectors.
2. Blind resolution is currently per Blind and capped at 32 in [`entity.rs`](../crates/oreak-core/src/entity.rs); the target is one Level-wide `1..128` pixels-per-cell setting. Migration must first verify all real existing Blinds agree.
3. Current decorator and entity IDs are distinct types and are only unique in their own sets. The target requires one global Level namespace.
4. The current [`DecoratorKind`](../crates/oreak-core/src/decorator.rs) has only Ice, Direction, and a one-key `KeyLocker`. Ice alone has a count-zero tombstone; generalize that into explicit enabled/disabled state, host metadata, endpoint collections, and persisted badge positions before extending the enum.
5. [`ApplyCommandRequest`](../crates/oreak-protocol/src/lib.rs) has no expected server sequence. A serialized mutex prevents simultaneous mutation but cannot reject a stale client intention or a stale Color/Sand Studio Apply.

## Classification gate for “new entity”

The extraction's parity model has exactly two primary placeables: Block and Blind/Pool. Treat a requested “new entity” as a **decorator** by default only when it has a Block/Blind host and behaves as a relation or modifier. A true third placeable requires an explicit product decision first, because it expands the exhaustive `PlaceableEntityKind` matches in core validation, transforms, hashing, legacy adaptation, rendering, selection, and tests.

## Recommended target seams

### Canonical core: `oreak-core`

Retain `LevelTimeline` as the command/audit engine. Extend its immutable snapshot with a versioned `LevelSettings` value object and canonical compatibility sidecar reference.

```rust
pub struct LevelSettings {
    duration_seconds: FiniteSeconds,
    pixels_per_cell: PixelsPerCell, // 1..=128, Level-wide
    palette_name: Option<PaletteName>,
    runtime_metadata: RuntimeMetadata,
}

pub struct Decorator {
    id: DecoratorId,
    enabled: bool,
    kind: DecoratorKind,
    badge_positions: EndpointPositions,
    compatibility: DecoratorCompatibility,
}

pub enum DecoratorKind {
    Ice { host: EntityId, blocking_count: u32 },
    Direction { host: EntityId, direction: DirectionMode },
    KeyLocker { locker: EntityId, keys: Vec<EntityId> },
    Glass { host: EntityId, blocking_count: u32 },
    HammerWall { wall: EntityId, hammers: Vec<EntityId> },
    Pipe { host: EntityId, mouth: PipeMouth, flow_rate: FiniteRate, stack_colors: Vec<ColorSlot> },
}

pub enum LevelCommand {
    // Existing commands omitted.
    UpdateCollectLayers { entity_id: EntityId, layers: Vec<CollectLayer> },
    SetDecoratorState { decorator_id: DecoratorId, enabled: bool },
    UpdateDecorator { decorator: Decorator },
    ReplaceBlindCanvas { entity_id: EntityId, source_canvas_hash: CanvasHash, patches: Vec<BlindTilePatch> },
}
```

The snippet names seams only: commands must remain typed and emit one timeline event with an exact inverse. Avoid a generic JSON-patch command because it weakens invariants, undo, blame, and legacy export ownership.

### Project configuration: `oreak-project` and server

Add immutable/fingerprinted project configuration for:

- color-profile slots C0..C15 (availability plus RGBA);
- collect defaults (radius, capacity, max speed);
- Color/Sand Studio source/profile fingerprints;
- later Pool-boundary defaults.

Make `oreak-core::LevelSettings.duration_seconds` the canonical authoring value. `oreak-project::LevelConfiguration` should become a project/catalog read model rather than a second mutable authority for duration.

### Color/Sand Studio: deterministic core plus private web session

Put canvas algorithms (mask stamping, connected-color fill, gradient sampling, rectangular move validation) behind deterministic `oreak-core` functions. Keep the temporary session, pointer interaction, panels, and capped local undo/redo in `apps/oreak-web`:

```rust
struct SandStudioSession {
    blind_id: EntityId,
    source_sequence: u64,
    source_canvas_hash: CanvasHash,
    draft: BlindCanvasDraft,
    history: BoundedHistory<StudioOperation, 64>,
}
```

Apply sends only the finalized target-Blind patch plus the captured expected sequence/hash. The server rejects stale input; the core guarantees non-target entities, decorators, guides, and Level metadata cannot change.

## Delivery sequence

### Phase 0 — contract and migration guardrails

1. Classify each requested “new entity” as a Block/Blind extension, decorator, or true third placeable; do not assume the extraction requires another primary entity.
2. Freeze representative core and legacy fixtures before changing serialized types.
3. Introduce explicit snapshot schema migration `v1 -> v2`; never silently deserialize v1 as v2 defaults without recording the migration path.
4. Preserve entity/decorator list order and add a derived ID lookup. Add a cross-kind global-ID validation test.
5. Add `expected_sequence` and stable `STALE_REVISION` RPC error handling before proposal-like workflows.

**Exit gate:** existing core, legacy, server, and WASM tests pass unchanged; v1 snapshots still load and re-save deterministically.

### Phase 1 — complete entity and color foundations

1. Add `LevelSettings` and migrate Blind resolution from per-Blind to level-wide, with a reject-on-disagreement migration rule.
2. Add source-shape ID/display name and full `CollectLayer` semantics: disabled color, inherited/default radius and capacity, independent lock, max speed, and ordering.
3. Add typed collection-layer commands and the Block inspector editor; preserve layer order through transforms, duplication, undo, and legacy round trips.
4. Add server-managed, fingerprinted color profiles and expose diagnostic palette availability in the web app. Raise Brush authoring range to C1..C15 without remapping unavailable colors.

**Exit gate:** a new Level produces the extracted defaults; a Block's complete ordered layer stack is editable collaboratively, undoable, serialized, and compatible with existing fixtures.

### Phase 2 — decorator model and compatibility

1. Generalize Ice's existing count-zero tombstone behavior into explicit enabled/disabled state for every decorator kind, then add badge-position storage before adding new kinds.
2. Upgrade Key/Locker to one Locker plus ordered unique Keys, including endpoint prune/last-key tombstone semantics.
3. Add Glass and Hammer/Wall with the host-kind and endpoint matrix enforced in core validation.
4. Add Pipe last: geometry/value objects, multiple Pipes per Blind, boundary/gravity/playable-lane/overlap validation, ordered stack colors, and current `eid/deco/m/fr/s` legacy encoding.
5. Update timeline inverse commands, rendering overlays/inspector controls, deletion transforms, and legacy golden fixtures in the same slice for each decorator kind.

**Exit gate:** every decorator acceptance item in [`ACCEPTANCE_SPEC.md`](LevelEditorExtraction/ACCEPTANCE_SPEC.md) §6 passes at core, RPC, web, and legacy boundaries. A disabled decorator restores the same identity, settings, and badge positions.

### Phase 3 — Color/Sand Studio

1. Implement the isolated per-Blind session and source-revision guard.
2. Deliver brush/eraser, color picker, connected-color Fill, then custom masks, profile-indexed gradient, and rectangular selection move/nudge/clear.
3. Commit one `ReplaceBlindCanvas` event only after successful stale validation; retain local history separately from collaborative timeline history.
4. Add the modal/panel workflow, conflict message, discard confirmation, and test fixtures for holes, guides, and irregular shapes.

**Exit gate:** Color/Sand Studio acceptance §10 passes: local history is capped at 64, Apply changes only one Blind in one revision, and remote edits produce a safe stale rejection.

### Phase 4 — deferred dependent features

Add capacity distribution/normalization after Pipe reserves and project defaults exist; then proposals for Divide/CAPX/resolution migrations; then durable storage/blob sharing, import/export UI, Sandbox parity, and batch migration.

## Validation and test plan

| Boundary | Required proof |
| --- | --- |
| Core | schema migration; order preservation; global IDs; shared resolution; collect order/default/lock/max-speed; every decorator host/endpoint/tombstone invariant; canvas non-target preservation. |
| Legacy | golden parse/export fixtures for all decorator kinds, tombstones, `cc` fourth value, `_led.pr`, raw unknown fields, and current Pipe `m/fr/s` format. |
| Protocol/server | expected-sequence success/stale retry behavior; duplicate idempotency behavior; server-authoritative actor/time; a stale studio Apply performs no mutation. |
| Web | entity creation and collect editing; disabled palette diagnosis; each decorator interaction; Color/Sand Studio Apply/discard/conflict path. |
| Regression | `cargo run -p bootstrap -- verify` plus focused native/WASM tests for every completed slice. |

## Non-goals for the first implementation increment

- CAPX import, Divide, batch migration, real-game launch, durable PostgreSQL/blob storage, and runtime-accurate sand simulation.
- Recreating Unity window layout or making the client authoritative.
- Auto-normalizing capacity as a side effect of save, import, Color/Sand Studio Apply, or an unrelated entity change.

## Primary risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Breaking existing serialized snapshots or Unity exports | Versioned migration, fixture-first golden tests, and adapter ownership boundaries. |
| Losing legacy entity order/unknown tokens | Preserve ordered compatibility envelopes; do not sort canonical lists during parsing or export. |
| Per-Blind resolution conversion corrupts artwork | Reject mixed-resolution documents, then resample only per logical cell with test fixtures for holes/guides. |
| Decorator implementation becomes a web-only feature | Core host/endpoint validation and typed timeline commands land before UI controls. |
| Large Studio payloads cause history/network pressure | Send bounded tile patches, validate source canvas hash, and defer content-addressed blobs to the durable-storage phase. |
| Color profile changes make a preview non-deterministic | Bind studio/proposal actions to profile/default fingerprints and reject stale Apply. |

## Implementation stop condition

Stop after Phases 0–3 only when the acceptance gates above pass across the core, protocol/server, legacy adapter, and browser. Do not claim full editor parity until the broader extracted acceptance matrix, especially import/export, proposals, transactions, and Sandbox isolation, also passes.
