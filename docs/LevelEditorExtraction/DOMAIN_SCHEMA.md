# Canonical Domain Schema

This document defines the technology-neutral, server-owned model. Names are descriptive rather than tied to any original language or framework.

## 1. Aggregate structure

```text
Level
  id: LevelId
  revision: positive integer or opaque revision token
  durationSeconds: finite number >= 0
  paletteName: optional non-blank string
  runtimeMetadata:
    difficulty: integer (recognized passthrough; not edited by current Level Editor)
    levelVersion: optional integer (recognized passthrough)
    description: optional string (recognized passthrough)
  board: Board
  pixelsPerCell: integer 1..128
  placeables: ordered list<Placeable>
  decorators: ordered list<Decorator>
  compatibilityEnvelope: opaque import/export preservation data
  createdAt, updatedAt, createdBy, updatedBy
```

### New-document defaults

The user-facing New command creates:

- board `8 x 8`;
- every cell Floor;
- `durationSeconds = 90`;
- no placeables or decorators;
- project-configured default pixels/cell, otherwise `32`;
- no source-file binding and no history before the first mutation.

The internal model may support a different constructor default, but a server API should expose the user-facing New behavior explicitly and never rely on hidden constructor defaults.

### Level-wide limits

- Board width and height: `1..256`.
- Board cell count: at most `65,536`.
- Pixels/cell: `1..128`.
- Duration: finite and non-negative on imported data. Interactive authoring normally requires at least one second.
- Placeable/decorator IDs are globally unique in the Level, not merely unique by kind.
- List order is semantically significant where stated. Import order must be retained for deterministic save and compatibility.
- Runtime root metadata (`difficulty`, optional version, optional description) is recognized and preserved even though the current editor does not expose authoring controls for it.

## 2. Coordinates and ordering

### Logical board coordinates

- Origin: bottom-left.
- Positive X: right.
- Positive Y: up.
- Dense index: `x + y * width`.
- Rows therefore enumerate bottom to top; cells within a row enumerate left to right.

### Local shape coordinates

- Every Shape is normalized to a tight local bounding rectangle.
- Local `(0,0)` is its bottom-left occupied bounding coordinate.
- World occupied cell = `placeable.origin + localCell`.

### Blind pixel coordinates

- Origin: bottom-left of the Blind's tight local bounding rectangle.
- Positive Y: up.
- Dense index: `x + y * pixelWidth`.
- Each occupied logical shape cell contributes an exact `pixelsPerCell x pixelsPerCell` paintable pixel square.
- Pixels under a footprint hole are non-paintable and must be Empty.

### Wire-coordinate warning

The legacy artwork payload stores rows top-to-bottom. Import/export must flip Y between wire pixels and canonical Blind pixels. This is an adapter concern, not a canonical-model concern.

## 3. Board

```text
Board
  width: integer 1..256
  height: integer 1..256
  cells: dense array<CellKind> of length width*height

CellKind = Floor | Wall
```

Rules:

- A placeable may occupy Floor only.
- A Floor-to-Wall edit is rejected if any placeable occupies that cell.
- Board resize remaps retained cells and all complete placeables by one common origin shift.
- A board resize is rejected if it would crop any placeable.
- Wall strokes are atomic commands and interpolate between sampled pointer cells; non-board input breaks interpolation.

## 4. Shape

```text
Shape
  size: { width, height }
  occupiedCells: sorted unique list<{x,y}>
```

Construction first deduplicates repeated input coordinates, tight-normalizes the remaining set, and sorts it row-major. Invariants after normalization:

- At least one occupied cell.
- Cells are unique.
- Tight bounds: at least one occupied cell touches every side of the declared bounding rectangle.
- Exactly one four-neighbour-connected component; diagonal-only contact is not connected.
- Maximum occupied-cell count: `64`.
- Block bounding dimensions: at most `8 x 8`.
- Blind bounding area: at most `64`; elongated shapes such as `1 x 64`, `2 x 32`, and `4 x 16` are valid.

Transforms:

- Clockwise rotation in a Y-up basis: `(x,y) -> (y, width-1-x)`; resulting bounds swap width/height.
- Horizontal flip: `(x,y) -> (width-1-x,y)`.
- Shapes have no authored pivot or wall-kick metadata.

## 5. Placeables

```text
Placeable = Block | Blind

common fields
  id: globally unique non-blank ID
  sourceShapeSampleId: optional library reference
  displayName: non-authoritative user-facing name
  origin: board coordinate
  shape: Shape
```

All occupied world cells must be inside the board, Floor, and disjoint from all other placeables.

### 5.1 Block

```text
Block extends Placeable
  kind: Block
  collectLayers: ordered list<CollectLayer>
```

A Block cannot own a Blind canvas or pool boundary configuration. Its ordered collect stack may contain at most `65,535` rows.

#### CollectLayer

```text
CollectLayer
  matchColor: integer -1..15
  radiusPixels: optional integer >= 0
  capacity: optional integer >= -1
  capacityLocked: boolean
  maximumCollectSpeed: optional finite number
```

Meaning:

- `matchColor = -1`: disabled row.
- `0..15`: enabled color index. Index order is semantic and must never be remapped or sorted.
- `radiusPixels = null`: resolve from project defaults; runtime fallback is `20`.
- `capacity = null`: resolve from project defaults; runtime fallback is `-1`.
- `capacity = -1`: Unlimited.
- `capacity = 0`: enabled but already exhausted/skipped for active-layer selection.
- `capacityLocked`: editor intent. It is independent per row and is not a runtime collect-row field.
- `maximumCollectSpeed = null`: resolve from project defaults; runtime fallback is `-1`.

The active collection layer is the first row, in authored order, that is enabled and whose resolved capacity is not zero. An active Unlimited row never completes and prevents later rows from becoming active.

### 5.2 Blind / Pool

```text
Blind extends Placeable
  kind: Blind
  canvas: BlindCanvas
  guides: GuideTopology
  boundary: PoolBoundary
  runtimePhysics: PoolRuntimePhysics
```

A Blind cannot own Block collect layers.

#### BlindCanvas

```text
BlindCanvas
  widthPixels = shape.width * level.pixelsPerCell
  heightPixels = shape.height * level.pixelsPerCell
  pixels: dense array<ColorIndexOrEmpty>

ColorIndexOrEmpty = Empty | integer 0..15 on import
Authorable painted colors = 1..15
```

Rules:

- Array length is exactly `widthPixels * heightPixels`.
- Non-paintable footprint-hole pixels are Empty.
- Existing legacy color `0` may be represented in canonical import state, but ordinary Brush authoring uses `1..15`.
- Artwork is independent of guide topology.
- Resolution changes resample nearest-neighbour inside each logical cell independently, preventing color bleed across cell boundaries or shape holes.

#### GuideTopology

Guides are unit edges on the complete Blind pixel lattice.

```text
GuideTopology
  horizontalEdges: bitset widthPixels * (heightPixels + 1)
  verticalEdges: bitset (widthPixels + 1) * heightPixels
```

Guides:

- partition Paint, Erase, Fill, Divide, and CAPX destination selection;
- remain visually white authoring geometry;
- do not recolor pixels;
- do not affect capacity connectivity, Sandbox gravity/collection, or runtime physics;
- may include retained boundary/hole edges, while footprint boundaries are always implicit traversal barriers.

#### PoolBoundary

```text
PoolBoundary
  paddingPixels: optional integer >= 0
  cornerRadiusPixels: optional integer >= 0
  authoredPolicy: boolean
```

- Null padding/corner values inherit project defaults.
- Explicit zero overrides a non-zero default.
- Effective playable pixels are resolved from the current boundary layout.
- Painted pixels excluded by padding/corners remain authored; edits do not erase them.
- Reports distinguish painted, playable, and excluded totals by color and exclusion reason.
- Invalid geometry blocks save. Excluded paint may exist in a draft/save, but gameplay preview rejects it when the pool has adopted authored validation.

#### PoolRuntimePhysics

These recognized runtime values live inside the Pool texture configuration. The current editor does not provide controls for them, but a lossless server must import, expose as passthrough/read-only metadata, preserve, and use relevant values during runtime-equivalent validation.

```text
PoolRuntimePhysics
  gravityDirectionX: optional integer
  gravityDirectionY: optional integer
  gravityPerStep: optional integer
  maxFallSpeed: optional integer
  slideFriction: optional finite number
  colorIndex: optional integer
  variantMin: optional integer
  variantMax: optional integer
```

When gravity is absent, runtime defaults downward. Pipe mouth validation must use the resolved Pool gravity. The original editor-side Pipe preview reconstructs a reduced Pool model and may miss imported custom gravity; the server specification intentionally closes that parity gap.

## 6. Decorators and relations

Every decorator has stable identity and may be active or a disabled tombstone.

```text
Decorator
  id: globally unique non-blank ID
  kind: DecoratorKind
  hostPlaceableId: PlaceableId
  enabled: boolean
  iconPositions: optional endpoint-local positions
  opaqueCompatibilityData
```

Host compatibility:

| Kind | Host | Additional data |
|---|---|---|
| Ice | Block | non-negative blocking count |
| Direction | Block | Horizontal or Vertical |
| KeyLocker | Block (Locker) | one or more Key Block IDs |
| Glass | Blind | non-negative blocking count |
| HammerWall | Blind (Wall) | one or more Hammer Block IDs |
| Pipe | Blind | mouth, flow rate, ordered stack colors |

Uniqueness:

- At most one non-Pipe decorator of a given kind per host.
- Multiple Pipes may target one Blind.
- Enabled Pipe mouths on one Blind cannot overlap the same simulation inlet pixels.

### 6.1 Counted decorators: Ice and Glass

```text
blockingCount: integer 0..2,147,483,647
```

- Default when newly enabled: `1`.
- An enabled decorator with count `0` normalizes into a disabled tombstone.
- Toggling back on restores default count `1`.
- Ice affects Sandbox movement and collection; Glass is authored/saved but not simulated by the editor Sandbox.

### 6.2 Direction

```text
direction: Horizontal | Vertical
```

In Sandbox, Horizontal allows horizontal movement only; Vertical allows vertical movement only. A disabled Direction tombstone has no effect.

### 6.3 Key / Locker

```text
hostPlaceableId: Locker Block ID
keyBlockIds: ordered/canonicalized unique list<BlockId>, at least one when enabled
endpointIconPositions: optional map<BlockId-or-LockerId, localPosition>
```

Rules:

- Locker and every Key must exist and be Blocks.
- No self-link.
- A Locker may have many Keys.
- A Block can be a Key for at most one enabled Key/Locker relation.
- A Block may simultaneously be a Locker in another relation, so chains are valid.
- Removing one Key prunes only that endpoint. Removing the last Key disables the relation as a reusable tombstone.
- Sandbox movement of the Locker remains allowed, but collection is blocked while any linked Key remains.

### 6.4 Hammer / Wall

```text
hostPlaceableId: Wall Blind ID
hammerBlockIds: ordered/canonicalized unique list<BlockId>, at least one when enabled
endpointIconPositions: optional map<BlindId-or-HammerId, localPosition>
```

Rules mirror Key/Locker role validation, except the host is a Blind and endpoints are Blocks. A Block can be one Hammer for one Wall and can independently also be a Key. The current editor Sandbox does not simulate Hammer/Wall gameplay.

### 6.5 Pipe

```text
Pipe
  hostPlaceableId: BlindId
  mouth:
    from: finite local-cell coordinate {x,y}
    to: finite local-cell coordinate {x,y}
  flowRateCellsPerSecond: finite number > 0
  stackColors: non-empty ordered list<integer 0..15>
```

Mouth rules:

- Non-zero, horizontal or vertical segment only.
- Coordinates are host-local logical-cell units and may be fractional at pixel-grid precision.
- Segment must lie continuously on one real footprint boundary facing outside the board's playable area.
- It cannot pour against the Blind's gravity from the bottom edge.
- After snapping to the Blind's pixel grid, at least one playable inlet lane must remain after padding/corner exclusions.
- Enabled mouths on one Blind may not share inlet simulation indices.

Each stack color represents one cell-area reserve: `pixelsPerCellX * pixelsPerCellY` grains. The mouth determines emission lanes; the ordered stack list determines pour order. Current editor Sandbox does not emit Pipes; gameplay preview is the parity path.

## 7. Decorator icon positions

```text
LocalIconPosition
  endpointPlaceableId
  x: finite local coordinate
  y: finite local coordinate
```

- Position belongs to decorator identity, not to the current visual instance.
- Whole badge must remain over occupied host footprint; holes/outside are invalid.
- Invalid release restores the previous position.
- Toggle off/on and save/reopen preserve positions.
- If a shape change invalidates a position, only that endpoint falls back to deterministic placement.
- Key/Locker and Hammer/Wall maintain separate positions per endpoint.

## 8. Shape sample library

The reusable Shape library is a project-level aggregate, not part of a Level revision.

```text
ShapeLibrary
  revision
  samples: ordered list<ShapeSample>

ShapeSample
  id: stable unique ID
  name: non-blank display name
  shape: Shape constrained to 8x8 designer bounds
```

- Rename changes only the library record.
- Update changes the stored name and geometry.
- Existing placed instances retain their copied name/shape.
- Delete does not delete placed instances.
- Reorder changes only library order.
- Library commands use their own concurrency/history domain, not Level Undo/Redo.

## 9. Derived data; never store as primary truth

The following should be computed and cacheable, but not independent authored fields:

- placeable world-cell set and bounds;
- painted/playable/excluded Blind pixels by color;
- guide-defined connected partitions;
- current Block active row;
- Block finite/unlimited capacity totals by color;
- Pipe reserves by color;
- global/per-color capacity deltas;
- badge fallback positions;
- Blind mip/coverage rendering data;
- normalization proposals;
- CAPX mapping and placement metrics;
- picture-grid migration proposals;
- Sandbox progress counters and scheduler state.

Caches must be keyed by the exact Level revision plus any project-default/catalog/profile fingerprints on which the calculation depends.

## 10. Canonical server serialization example

This example is intentionally readable and is not the legacy game wire format.

```json
{
  "id": "level-42",
  "revision": 17,
  "durationSeconds": 90,
  "paletteName": "Default",
  "runtimeMetadata": {"difficulty":0,"levelVersion":1,"description":null},
  "board": {
    "width": 8,
    "height": 8,
    "cells": ["Floor", "Floor", "Wall"]
  },
  "pixelsPerCell": 32,
  "placeables": [
    {
      "kind": "Block",
      "id": "block-a",
      "origin": {"x": 1, "y": 1},
      "shape": {"width": 2, "height": 1, "occupiedCells": [[0,0],[1,0]]},
      "collectLayers": [
        {
          "matchColor": 3,
          "radiusPixels": null,
          "capacity": 120,
          "capacityLocked": false,
          "maximumCollectSpeed": null
        }
      ]
    },
    {
      "kind": "Blind",
      "id": "pool-a",
      "origin": {"x": 4, "y": 1},
      "shape": {"width": 1, "height": 1, "occupiedCells": [[0,0]]},
      "canvasRef": "blob:sha256:...",
      "guides": {"horizontalRef": "...", "verticalRef": "..."},
      "boundary": {"paddingPixels": null, "cornerRadiusPixels": 0, "authoredPolicy": true},
      "runtimePhysics": {"gravityDirectionX":0,"gravityDirectionY":-1}
    }
  ],
  "decorators": [
    {
      "kind": "Pipe",
      "id": "pipe-a",
      "hostPlaceableId": "pool-a",
      "enabled": true,
      "mouth": {"from": [0,1], "to": [1,1]},
      "flowRateCellsPerSecond": 0.5,
      "stackColors": [3,3,7]
    }
  ]
}
```
