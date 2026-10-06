# Validation and Transaction Semantics

## 1. Validation layers

Validation should be staged so clients receive precise, stable error codes and no partial mutation.

### Layer A — transport and syntax

- authenticated/authorized request;
- payload/file size;
- valid JSON/binary framing;
- duplicate-property, nesting, trailing-data checks;
- required command fields and finite numbers.

### Layer B — value objects

- coordinates, dimensions, colors, counts, rates;
- Shape normalization/connectivity;
- canvas lengths and color domain;
- guide bitset lengths;
- Pipe mouth segment shape;
- CAPX and migration limits.

### Layer C — entity-local rules

- Block has collect layers and no canvas;
- Blind has canvas matching shape/resolution and no collect rows;
- boundary values non-negative;
- counted decorator fields;
- relation endpoint lists nonempty/unique when enabled.

### Layer D — aggregate invariants

- globally unique IDs;
- every placeable in board/on Floor;
- no occupied-cell overlap;
- one shared pixels/cell;
- decorator host types and endpoint existence;
- relation ownership uniqueness;
- Pipe geometry and non-overlap;
- adopted pool-boundary validity.

### Layer E — command-specific rules

- source revision/proposal token freshness;
- selection compatibility;
- active interaction lock;
- no crop/overlap/wall conflict;
- capacity arithmetic feasibility;
- external source/catalog/default fingerprints;
- file path and batch transaction safety.

### Layer F — export/runtime verification

- legacy JSON can be built;
- rebuild can be reopened and semantically compared;
- gameplay-specific pool loss policy;
- runtime decoder/verifier accepts payload.

A command commits only when every required layer passes.

## 2. Error model

Use machine-readable errors with optional entity/field/geometry detail.

```json
{
  "code": "PLACEABLE_OVERLAP",
  "message": "The requested Block overlaps another placeable.",
  "levelId": "level-42",
  "sourceRevision": 17,
  "entityIds": ["block-a", "pool-b"],
  "field": "origin",
  "cells": [[4,3]],
  "retryable": false
}
```

Recommended code families:

- `PARSE_*`, `LIMIT_*`, `SCHEMA_*`
- `STALE_REVISION`, `STALE_PROPOSAL`, `STALE_EXTERNAL_SOURCE`
- `INVALID_SHAPE`, `INVALID_CANVAS`, `INVALID_GUIDE`
- `OUT_OF_BOARD`, `WALL_CONFLICT`, `PLACEABLE_OVERLAP`
- `INVALID_DECORATOR_HOST`, `RELATION_CONFLICT`
- `INVALID_PIPE_MOUTH`, `PIPE_OVERLAP`
- `INVALID_POOL_BOUNDARY`, `EXCLUDED_AUTHORED_SAND`
- `CAPACITY_CONFLICT`, `CAPACITY_OVERFLOW`, `MISSING_COLOR_TARGET`
- `IMPORT_UNSUPPORTED`, `COMPATIBILITY_CONFLICT`
- `PERSISTENCE_CONFLICT`, `PERSISTENCE_FAILED`, `ROLLBACK_INCOMPLETE`

Validation errors are expected domain results, not internal server faults.

## 3. Optimistic concurrency

Every Level command includes an expected revision.

```text
execute(levelId, expectedRevision, command, idempotencyKey)
```

Rules:

- Mismatch returns `STALE_REVISION` with current revision and optional compact change summary.
- Never silently apply a command to a newer revision.
- Repeated idempotency key returns the original result.
- Successful mutation creates exactly one new revision and one audit/history record.
- No-op/invalid/cancel creates no revision.

For multi-user editing, broadcast committed commands/revisions. Clients with open pointer interactions must cancel or explicitly recompute when a remote revision arrives.

## 4. Proposal lifecycle

Long or high-impact calculations return proposals:

```text
Proposal
  id
  type
  levelId
  sourceRevision
  normalizedInputs
  externalFingerprints
  diagnostics
  candidateSummary or candidateBlobRef
  expiresAt
```

Used for:

- Divide;
- CAPX mapping/placement;
- Distribution;
- Normalize;
- pixels-per-cell migration;
- batch migration;
- optionally complex merge.

Apply checks:

- proposal exists/not expired;
- same Level revision;
- same project defaults/profile/catalog/source fingerprints;
- candidate remains valid;
- operation is not a no-op.

Apply commits once and consumes the proposal. Cancel consumes it without revision. Changing source inputs invalidates dependent proposals.

## 5. Command atomicity

A command is all-or-nothing for the canonical Level.

Examples:

- group move/rotate/flip rejects the entire group if one result is invalid;
- Blind resize changes shape, origin, artwork, guides, Pipes, and decorator positions together;
- placeable deletion updates relation endpoints/tombstones in the same revision;
- Normalize updates all planned capacities/locks together;
- Sand Studio Apply replaces one canvas or nothing;
- current-level picture-grid migration changes resolution, all canvases, guides, and affected capacities together.

Duplicate drag is a deliberate exception in product semantics: each requested copy is independently valid and the command commits the valid subset as one revision. The response must list created and rejected source IDs so this partial-by-design behavior is explicit.

## 6. Undo and redo on a server

Recommended model: append-only revisions plus per-user/session cursor.

Two valid strategies:

### Inverse-command strategy

Store normalized command and generated inverse. Suitable when every command has a stable inverse and compatibility envelope changes are captured.

### Snapshot-revision strategy

Store immutable canonical snapshot references. Undo creates a new revision whose content equals an earlier snapshot; redo similarly creates a new revision. This matches collaboration/audit systems better than moving a global mutable cursor.

Requirements:

- preserve action labels;
- cap interactive client history view to 80 while retaining server audit according to policy;
- Undo/Redo cannot target transient Sandbox history;
- remote edits must not be erased implicitly; require explicit revert semantics in collaborative mode.

## 7. Dirty and baseline semantics

A document session tracks:

- opened revision/source;
- last accepted saved/exported baseline;
- current authored revision;
- compatibility envelope/source fingerprint.

Dirty if canonical current differs from accepted baseline, including duration/palette, even when source file is not bound.

A successful export build does not make the session clean. Only accepted durable persistence does.

## 8. Single-file save

Safe save sequence:

1. Ensure no active authoring gesture.
2. Publish pending valid delayed fields.
3. Validate canonical Level and save-specific policies.
4. Compare current source fingerprint with baseline.
5. Build legacy JSON in memory.
6. Optionally reopen and semantic-compare build.
7. Write a temporary file in destination directory.
8. Flush and verify bytes/hash.
9. Atomically replace destination.
10. Re-read fingerprint if required.
11. Advance compatibility envelope and saved baseline.

If source changed externally, ordinary Save must fail with conflict; user can reopen, fork, or Save As. A clean ordinary Save should not rewrite unchanged salted payloads.

## 9. Recovery

Recovery protects unsaved authored state across process/domain reload.

- Version recovery schema.
- Store source binding/fingerprint, preservation template, saved baseline, and current draft.
- Enforce total size limit.
- Clear record before attempting restore to avoid a corrupt restore loop.
- Validate all canonical invariants after decode.
- A valid recovery draft takes priority over reopening last saved file.
- Reset Undo/Redo and report that reset.
- Old recovery schemas are ignored or migrated explicitly; never guess.

For a web/server editor, use periodic draft checkpoints with TTL and explicit “restore/fork/discard” UX.

## 10. Capacity validation

### Effective sand

Per color:

```text
playableBlindSand = painted - excludedByBoundary
pipeReserve = sum(stackCellArea for every active Pipe stack of that color)
sandTarget = playableBlindSand + pipeReserve
```

Invalid boundary geometry blocks any calculation that would otherwise treat the pool as empty.

### Normalize conflicts

Reject proposal when any color has:

- sand target but no eligible same-color Block row;
- locked Unlimited row under preserve-lock mode;
- locked finite subtotal greater than target;
- insufficient integer headroom/range;
- all adjustable rows absent but subtotal differs;
- overflow.

Do not mutate artwork, shapes, placement, row colors/order, decorators, duration, or unknown compatibility fields.

### Distribution conflicts

- at least one Pool/group target required;
- nonempty selected region cannot receive zero total weight;
- locked rows remain fixed;
- unmatched colors are ignored/reported according to workflow, not silently recolored.

## 11. Pool-boundary policies

Three distinct outcomes must remain distinct:

1. **Valid and no excluded paint** — save/play allowed.
2. **Valid geometry with excluded authored paint** — save draft allowed with loss report; adopted gameplay preview rejects.
3. **Invalid boundary geometry** — save/normalize/sandbox/play validation fails.

Boundary edits never erase artwork and never automatically change Block capacity.

## 12. CAPX transaction safety

Fingerprint all dependencies:

- external file bytes or project asset deep content;
- color-profile availability and colors;
- transparency options;
- target Level revision;
- target Blind and captured partition identity.

Color Review approval does not authorize a later modified source. Apply revalidates every dependency, then changes one Blind canvas in one revision.

## 13. Picture-grid batch transaction

Preparation may classify files as eligible, unchanged, or skipped. Partial success is allowed only at this classification stage.

Apply treats all eligible changed files as one transaction:

1. Revalidate catalog fingerprint and mappings.
2. Revalidate default-configuration fingerprint.
3. Lock eligible files.
4. Verify source hashes from preview.
5. Create and verify backups for changed files only.
6. Write each result and verify its hash/semantic reopen.
7. On any failure, restore touched files in reverse order.
8. Re-hash restored files and report verified/incomplete rollback.

Limits inherited from current behavior:

- 64 MiB per file;
- 256 MiB total source JSON per batch;
- 4,194,304 dense source pixels and target pixels per level;
- 16,777,216 retained dense pixels per batch.

Configured paths must satisfy the deployment's trusted-root and symlink policy. A server should use managed object storage rather than allowing arbitrary filesystem paths.

## 14. Sandbox transaction isolation

Sandbox starts from one authored revision and creates a private branch:

```text
SandboxSession
  sourceLevelRevision
  currentSimulationSnapshot
  progressCounters
  privateHistory
  schedulerState
  selection
```

- No Sandbox command can update authored Level.
- Restart rebases only by creating a fresh Sandbox session from latest authored revision.
- Leaving deletes/expires the branch.
- Gameplay PLAY always reads authored revision, never Sandbox branch.
- Simulation history restores pixels, placeables, decorators, and counters atomically.

## 15. Security and abuse limits

A server implementation should enforce:

- authenticated project/level/library access;
- per-command payload and decoded-allocation limits;
- checked integer arithmetic for dimensions/capacity/reserve;
- bounded proposal count and expiry;
- bounded simulation CPU/tick budgets;
- upload MIME/extension plus content validation;
- no arbitrary server filesystem reads from client-supplied paths;
- audit trail for save, export, batch migration, and gameplay publication;
- rate limits on expensive Divide/InsideShape/migration calculations;
- content-addressed artwork blobs with tenant/project isolation.
