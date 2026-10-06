# Server Blueprint and API Contract

This is a recommended architecture, not a mandate for a particular language or database.

## 1. Service boundaries

```text
API Gateway / Auth
  |
Level Command Service -------- Revision/Event Store
  |                                   |
  +-- Domain Validator                +-- Snapshot Store
  +-- Proposal Service                +-- Artwork Blob Store
  +-- Derived Analysis                +-- Audit Log
  +-- Sandbox Service
  +-- Import/Export Adapter ------ Legacy JSON/Object Storage
  +-- Batch Job Service ---------- Catalog/Profile/Defaults providers
```

### Level Command Service

Owns canonical Level mutations, ID generation, aggregate validation, optimistic concurrency, and revision creation.

### Proposal Service

Owns expensive/staged computations and stale-safe Apply tokens: Divide, CAPX, normalization, distribution, migrations.

### Import/Export Adapter

Owns the compact runtime JSON, DataCodec, unknown-field preservation, round-trip comparison, and runtime verification.

### Sandbox Service

Owns temporary simulation branches, deterministic ticks, private history, expiry, and optional streamed frames.

### Batch Job Service

Owns catalog snapshots, migration preparation, backups, transaction/rollback, job progress, and reports.

### Project Configuration Providers

Version/fingerprint:

- collection defaults;
- palette/color profiles;
- gameplay level catalog;
- runtime verifier version;
- shape library.

Derived proposals must include these fingerprints.

## 2. Persistence model

Recommended logical tables/collections:

```text
projects
levels
level_revisions
level_events
level_compatibility_envelopes
artwork_blobs
proposal_records
sandbox_sessions
shape_libraries
shape_library_revisions
external_source_bindings
batch_jobs
batch_job_files
audit_records
```

### Level revision

```text
level_id
revision_number
parent_revision
canonical_document_json
compatibility_envelope_ref
content_hash
command_type
command_payload_normalized
actor_id
created_at
```

Large dense canvases/guides should be content-addressed blobs referenced by snapshot. This allows structural sharing across revisions and avoids rewriting every canvas for a small property edit.

### Blob encoding

Internally prefer a simple versioned binary form:

```text
header: schema version, width, height, color encoding
payload: dense bytes or chunked/compressed tiles
hash: SHA-256 of canonical decoded content
```

Do not use legacy DataCodec as the primary blob representation. Convert only at the adapter boundary.

## 3. Resource API

All mutation responses include new revision, normalized command result, diagnostics, and emitted event IDs.

### Levels

```http
POST   /projects/{projectId}/levels
GET    /projects/{projectId}/levels/{levelId}
GET    /projects/{projectId}/levels/{levelId}/revisions/{revision}
DELETE /projects/{projectId}/levels/{levelId}
```

Create request:

```json
{
  "name": "Level 42",
  "template": "blank",
  "pixelsPerCell": 32,
  "idempotencyKey": "uuid"
}
```

### Generic command endpoint

```http
POST /projects/{projectId}/levels/{levelId}/commands
If-Match: "revision-17"
Idempotency-Key: uuid
```

```json
{
  "type": "MovePlaceables",
  "payload": {
    "placeableIds": ["block-a","block-b"],
    "delta": {"x":1,"y":0}
  }
}
```

Response:

```json
{
  "levelId": "level-42",
  "previousRevision": 17,
  "revision": 18,
  "changed": true,
  "command": {"type":"MovePlaceables"},
  "diagnostics": [],
  "events": ["evt-..."]
}
```

A generic endpoint keeps command semantics uniform; generated client SDKs may expose typed helpers.

## 4. Command catalog

### Level and board

- `SetDuration`
- `SetPalette`
- `SetBoardCellKinds`
- `ResizeBoard`
- `SetPixelsPerCell` only through accepted migration proposal

### Placeables

- `PlaceFromShape`
- `MovePlaceables`
- `DuplicatePlaceables`
- `RotatePlaceablesClockwise`
- `FlipPlaceablesHorizontally`
- `MergePlaceables`
- `DeletePlaceables`
- `ReplacePlaceableShape`
- `ResizeBlindEdge`

### Block layers

- `InsertCollectLayer`
- `RemoveCollectLayer`
- `MoveCollectLayer`
- `UpdateCollectLayer`
- `SetCollectLayerCapacityLock`
- `RecolorActiveLayer`

### Decorators

- `ToggleDecorator`
- `SetCountedDecoratorCount`
- `SetDirection`
- `AddRelationEndpoint`
- `RemoveRelationEndpoint`
- `DisableRelation`
- `MoveDecoratorEndpointIcon`
- `CreatePipe`
- `UpdatePipe`
- `SetPipeEnabled`

### Blind artwork/guides

- `PaintStroke`
- `EraseStroke`
- `FillRegion`
- `AddGuideEdges`
- `ReplaceBlindCanvas`
- `ApplyCapxPlacement` through proposal
- `ApplySandStudioCanvas`

### Capacity workflows

- `ApplyDistribution` through proposal
- `ApplyNormalization` through proposal

Every command payload should describe semantic intent, not screen-space pointer samples. Clients may submit rasterized stamps/edges if the server validates and canonicalizes them, but authoritative grid/pixel effects must be deterministic.

## 5. Proposal API

```http
POST   /projects/{projectId}/levels/{levelId}/proposals/{type}
GET    /projects/{projectId}/levels/{levelId}/proposals/{proposalId}
POST   /projects/{projectId}/levels/{levelId}/proposals/{proposalId}:apply
DELETE /projects/{projectId}/levels/{levelId}/proposals/{proposalId}
```

Common create request:

```json
{
  "sourceRevision": 17,
  "inputs": {},
  "dependencyFingerprints": {}
}
```

Common response:

```json
{
  "proposalId": "prop-...",
  "type": "NormalizeCapacities",
  "sourceRevision": 17,
  "status": "ready",
  "canApply": true,
  "expiresAt": "...",
  "summary": {},
  "diagnostics": [],
  "previewDelta": []
}
```

Types and special inputs:

### `DividePartition`

- Blind ID, seed pixel, weights, metric, direction.
- Returns generated guide edges, achieved counts, fit quality, direction.

### `MapCapx`

- Upload/source reference, palette-zero policy, alpha cutoff, profile fingerprint.
- Returns mapping review and mapped-image blob.

### `PlaceCapx`

- Accepted map proposal, Blind/partition, fit, rectangle/scale, write mode.
- Returns artwork ghost mask and pixel metrics.

### `DistributeCapacity`

- selected Blocks, Pools/groups, weights, lock policy.

### `NormalizeCapacities`

- preserve locks or explicit allow-locked mode.
- Returns per-color before/after and unlock list.

### `MigratePixelResolution`

- target pixels/cell.
- Returns canvas/capacity deltas and blockers.

## 6. Artwork upload API

```http
POST /projects/{projectId}/uploads/capx
POST /projects/{projectId}/uploads/legacy-level-json
```

Uploads are quarantined until content validation completes. Return an opaque source ID and SHA-256, never a server filesystem path.

```json
{
  "sourceId": "src-...",
  "sha256": "...",
  "kind": "capx",
  "metadata": {"width":64,"height":64,"paletteCount":8},
  "validation": {"valid":true,"diagnostics":[]}
}
```

## 7. Import/export API

```http
POST /projects/{projectId}/levels:importLegacy
POST /projects/{projectId}/levels/{levelId}:buildLegacy
POST /projects/{projectId}/levels/{levelId}:publishLegacy
GET  /projects/{projectId}/levels/{levelId}/exports/{exportId}
```

### Import

Request includes source upload ID and optional source binding. Response creates a canonical Level only after complete parse/validation.

### Build

Pure operation: returns or stores candidate bytes, diagnostics, content hash, runtime verification result, and source revision. It does not mark anything saved/published.

### Publish

Checks source revision and destination binding fingerprint, persists atomically, then records accepted baseline/publication audit.

## 8. Sandbox API

```http
POST   /projects/{projectId}/levels/{levelId}/sandboxes
GET    /projects/{projectId}/sandboxes/{sandboxId}
POST   /projects/{projectId}/sandboxes/{sandboxId}/commands
POST   /projects/{projectId}/sandboxes/{sandboxId}:tick
POST   /projects/{projectId}/sandboxes/{sandboxId}:restart
DELETE /projects/{projectId}/sandboxes/{sandboxId}
```

Create binds exact authored revision and project-default fingerprint.

Sandbox commands:

- move/nudge/analog path;
- delete Block;
- pause/resume;
- manual tick;
- undo/redo.

Server may auto-tick and stream deltas, or clients may request deterministic ticks. For collaborative authoring, keep Sandbox private by default.

Sandbox response should include:

- simulation sequence number;
- changed pixel runs/blob hashes;
- moved/removed entities;
- decorator changes;
- per-row counters/active row;
- running/paused/settled/error state.

## 9. Batch-job API

```http
POST /projects/{projectId}/jobs/picture-grid-migration:preview
GET  /projects/{projectId}/jobs/{jobId}
POST /projects/{projectId}/jobs/{jobId}:apply
POST /projects/{projectId}/jobs/{jobId}:cancel
GET  /projects/{projectId}/jobs/{jobId}/report
```

Job states:

```text
queued -> preparing -> ready
ready -> applying -> succeeded
ready/applying -> failed
applying -> rolling_back -> failed_rolled_back | failed_rollback_incomplete
any nonterminal -> canceled where safe
```

Per-file states:

- eligible-changed;
- eligible-unchanged;
- skipped with reason;
- backed-up;
- written/verified;
- restored/verified;
- rollback-incomplete.

## 10. Shape-library API

```http
GET  /projects/{projectId}/shape-library
POST /projects/{projectId}/shape-library/commands
```

Commands use library `If-Match` revision and include create, rename, update geometry, reorder, delete. They never mutate Levels containing copied instances.

## 11. Events and realtime collaboration

Use WebSocket/SSE topic per project/Level.

Events:

```text
LevelRevisionCreated
PlaceablesChanged
CanvasBlobChanged
DecoratorChanged
ProposalInvalidated
ProposalReady
SandboxFrameAdvanced
SandboxEnded
BatchJobProgress
PublicationSucceeded
ExternalSourceConflict
ProjectDefaultsChanged
PaletteProfileChanged
CatalogChanged
ShapeLibraryRevisionCreated
```

Clients receive revision and changed resource IDs/blob refs. Avoid broadcasting whole dense canvases for every brush sample; broadcast committed run diffs or new content hashes.

Presence/ephemeral collaboration may separately broadcast cursors, selections, active tool, and preview ghosts. Presence must never mutate canonical Level.

## 12. Project defaults and fingerprints

```http
GET /projects/{projectId}/level-editor-config
```

Return:

- default pixels/cell;
- Block collection defaults;
- pool boundary defaults;
- color profiles C0..C15 with availability/colors;
- catalog fingerprint/summary;
- runtime verifier version;
- feature capabilities.

Every proposal/job records only the fingerprints it actually depends on. A relevant change invalidates it.

## 13. Authorization model

Suggested roles:

- Viewer: read revisions/previews.
- Author: edit Levels and private Sandbox.
- Publisher: publish runtime JSON/gameplay levels.
- Library Maintainer: mutate Shape library.
- Migration Operator: preview/apply batch jobs.
- Administrator: project defaults/catalog/integration configuration.

Gameplay publication and batch migration should be separately auditable privileged actions.

## 14. Deployment guidance

### Initial extraction phases

1. Implement canonical schema, validation, revision store, and read-only legacy import.
2. Add lossless legacy export with round-trip golden tests.
3. Add basic board/placeable/collect/decorator commands.
4. Add artwork blobs and Brush/Sand Studio commands.
5. Add proposals: Divide, CAPX, distribution, normalization, resolution migration.
6. Add Sandbox as an isolated service.
7. Add batch migration and runtime publication integrations.
8. Add collaborative presence and conflict-aware UX.

### Stop condition for parity

Do not claim replacement parity until the acceptance matrix passes against representative legacy fixtures, including unknown-field/token preservation, stale proposal rejection, atomic save/rollback, current Pipe schema, max-speed collect rows, boundary loss policy, and Sandbox isolation.
