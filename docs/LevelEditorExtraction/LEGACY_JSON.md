# Legacy Runtime JSON Contract

The server should store the canonical model described in `DOMAIN_SCHEMA.md` and use this format only through an import/export compatibility adapter.

## 1. Preservation principle

Legacy files may contain unknown root fields, unknown entity fields, compact token choices, future runtime data, and editor metadata. Import/export must be lossless outside fields whose owned semantic value changed.

The adapter must:

1. parse into a complete candidate before replacing any session state;
2. retain the original root object, entity order, owned-entity payload templates, and raw compact tokens;
3. map owned fields into the canonical model;
4. on export, clone the original envelope and patch only changed owned fields;
5. preserve unknown properties verbatim at the JSON-token level where possible;
6. accept a built export as the new baseline only after durable persistence succeeds.

Do not deserialize and reserialize the whole file into a clean object graph; that would destroy compatibility data.

## 2. Root object

```json
{
  "v": 1,
  "desc": "Optional runtime description",
  "dur": 90,
  "dif": 0,
  "pal": "Default",
  "bes": [8, 8],
  "bdat": "<DataCodec string>",
  "entitites": [
    ["block", {"eid":"block-a", "g":{}, "cc":[]}],
    ["pool", {"eid":"pool-a", "g":{}, "stc":{}}]
  ],
  "_led": {}
}
```

The misspelling `entitites` is canonical runtime compatibility and must not be corrected.

| Field | Meaning | Validation / preservation |
|---|---|---|
| `v` | Optional Level version | Recognized runtime metadata; current editor does not edit it and must preserve it. |
| `desc` | Optional Level description | Recognized runtime metadata; current editor does not edit it and must preserve it. |
| `dur` | Whole-level seconds | Finite, non-negative imported value. Untouched valid token should remain raw. |
| `dif` | Difficulty integer | Recognized runtime metadata; current editor does not edit it and must preserve it. |
| `pal` | Optional palette name | Missing/blank maps to no explicit palette. |
| `bes` | `[width,height]` | Each 1..256 and product <=65,536. |
| `bdat` | Board Floor mask | DataCodec-packed boolean array of exactly width*height; true=Floor. |
| `entitites` | Ordered entity envelopes | Preserve order and unknown envelopes. |
| `_led` | Editor compatibility metadata | Optional; owned subfields described below. |

Parser safety:

- root must be one JSON object;
- reject duplicate object properties;
- reject trailing JSON;
- maximum nesting depth 64;
- comments may be ignored;
- no automatic date coercion.

A server should additionally enforce a request/file size limit. Existing editor flows use 64 MiB for one Level JSON and 96 MiB for a recovery record.

## 3. Entity envelope

Every known runtime entity is:

```json
["marker", { "eid": "globally-unique-id", "...": "payload" }]
```

Known markers:

- `block`
- `pool`
- `ice`
- `direction`
- `key-locker`
- `glass`
- `hammer-wall`
- `pipe`

Rules:

- Envelope has marker plus object payload.
- `eid` is nonempty and globally unique across placeables and decorators.
- Entity order cannot be assumed to satisfy references; use two-pass resolution.
- Unknown properties inside supported payloads are preserved. An unsupported entity marker is rejected by the current compatibility contract; report a structured import error rather than partially loading or silently dropping it.

## 4. DataCodec

Several compact values use a runtime codec conceptually represented as:

```text
base64(salt) : base64(xorEncodedPayload) : CRC32
```

The payload may be compressed. The salt's first eight bytes are runtime salt; editor-only versioned suffixes may follow. Runtime decoders tolerate the suffix while decoding the same payload bytes.

Adapter requirements:

- validate base64, checksum, decompression, decoded length, and metadata suffix structure;
- never guess malformed metadata versions or lengths;
- preserve an unchanged raw codec token exactly;
- generate a fresh valid codec token only when the owned value changes;
- isolate codec logic behind one tested binary adapter.

## 5. Board mask

`bdat` decodes to exactly `width*height` boolean values in canonical bottom-left row-major order.

```text
index = x + y * width
true  = Floor
false = Wall
```

## 6. Placeable geometry

Both Block and Pool carry:

```json
"g": {
  "r": [originX, originY, width, height],
  "d": "<DataCodec occupancy mask>"
}
```

- `g.r` declares world origin and tight local bounds.
- `g.d` decodes to row-major occupied-cell booleans.
- Shape must be nonempty, tightly bounded, and four-connected.
- Block bounds <=8x8.
- Pool bounding area <=64.
- Occupancy suffix may carry editor guide or Block-lock metadata.

### Editor metadata in `g.d` salt

- Blind guide topology has a versioned marker and lattice bitsets.
- Block capacity locks use `SBCL` metadata.
  - v1: legacy first-row lock only.
  - v2: config-count-bound lock bitset, one bit per `cc` row.
- An ordinary legacy eight-byte salt means all rows unlocked and no editor suffix.
- Malformed marker/version/config count/padding/suffix length rejects import.

## 7. Block payload

Minimal shape:

```json
[
  "block",
  {
    "eid": "block-a",
    "g": {"r":[1,1,2,1], "d":"..."},
    "cc": [
      [3, null, 120, null],
      [-1],
      [7, 24, -1, 2.5]
    ]
  }
]
```

### `cc` row

The `cc` array may contain at most `65,535` rows. Current contract supports up to four semantic values per row:

```text
[matchIndex, collectRadiusPixels, capacity, maximumSandCollectSpeed]
```

A complete row may be `null`, which imports as a disabled/default row.

| Index | Domain |
|---|---|
| 0 | `-1` disabled, null/default, or color `0..15` |
| 1 | null/default or integer >=0 |
| 2 | null/default, `-1` Unlimited, or integer >=0 |
| 3 | null/default or finite number |

- Row order is inner/current first.
- Null/missing values resolve through project defaults; runtime fallbacks are radius `20`, capacity `-1`, max speed `-1`.
- Lock state is not a `cc` field; it lives in `g.d` salt metadata.
- When patching a changed row, preserve any unknown row tail if compatibility policy allows; current runtime rejects rows longer than four, so a replacement should surface conflict rather than silently truncate.

## 8. Pool / Blind payload

```json
[
  "pool",
  {
    "eid": "pool-a",
    "g": {"r":[4,1,1,1], "d":"..."},
    "stc": {
      "spp": 2,
      "spcr": 4,
      "spauth": true,
      "gdx": 0,
      "gdy": -1,
      "gps": null,
      "mfs": null,
      "sfr": null,
      "col": null,
      "lmin": null,
      "lmax": null,
      "c": {
        "r": [32,32],
        "cmp": true,
        "data": "<compressed DataCodec>"
      }
    }
  }
]
```

### Boundary fields

- `spp`: null/missing or non-negative integer padding pixels.
- `spcr`: null/missing or non-negative integer corner radius pixels.
- `spauth`: null/missing/false for legacy policy; true opts into authored boundary validation.

Null means inherit project defaults; explicit zero overrides.

### Runtime Pool physics fields

The same `stc` object also recognizes fields not edited by the current Level Editor:

| Field | Meaning |
|---|---|
| `gdx`, `gdy` | optional gravity direction components |
| `gps` | optional gravity per step |
| `mfs` | optional maximum fall speed |
| `sfr` | optional slide friction |
| `col` | optional runtime color index |
| `lmin`, `lmax` | optional visual variant range |

A lossless adapter preserves these fields. Runtime-equivalent Pipe validation must resolve `gdx/gdy`; when gravity is absent the runtime defaults downward. The original editor's reduced Pipe validation model may not reflect imported custom gravity, so a server implementation should treat this as a parity fix, not repeat the omission.

### Canvas `stc.c`

| Field | Meaning |
|---|---|
| `r` | `[pixelWidth,pixelHeight]` |
| `cmp` | `true` for compressed payload |
| `data` | compressed DataCodec or null blank representation |

Rules:

- Dimensions must equal one common integer pixels/cell multiplied by shape width/height.
- All non-placeholder Pools in a Level must imply the same pixels/cell.
- Exact `r:[1,1], data:null` is a compact placeholder and does not participate in resolution inference.
- Blank real canvas may use null when unambiguous.
- Blank one-pixel real canvas must encode one `00` byte so it is not mistaken for the placeholder.
- Decoded payload is exactly width*height bytes.
- Wire rows are top-to-bottom; canonical rows are bottom-to-top.
- Byte `0x00` is Empty.
- Painted byte uses high nibble as color index `1..15`; low visual-variant nibble is tolerated on read.
- New writes use `(colorIndex << 4) | 12`.
- Reject painted bytes outside the irregular footprint.
- Preserve unchanged raw data exactly.

## 9. Level pixel-resolution metadata

Optional:

```json
"_led": {
  "pr": 32
}
```

- Must be integer `1..128`.
- Must agree with every real Pool canvas.
- Preserves an explicit resolution even with zero Pools or only compact placeholders.
- Without metadata and without inferable canvas, default to 32.
- Conflicts reject import; never silently resample on open.

## 10. Active decorators

### Ice and Glass

```json
["ice",   {"eid":"ice-a",   "deco":"block-a", "count":1}]
["glass", {"eid":"glass-a", "deco":"pool-a",  "count":2}]
```

- `deco` references compatible existing host.
- `count` is non-negative integer.
- Count zero becomes disabled canonical state and is exported as tombstone rather than active runtime entity.

### Direction

```json
["direction", {"eid":"dir-a", "deco":"block-a", "dir":12}]
```

Canonical exports use numeric flags:

- Horizontal: `12` (`0xC`).
- Vertical: `3` (`0x3`).

Importer also accepts exact legacy strings `"Horizontal"` and `"Vertical"`.

### Key / Locker

```json
["key-locker", {
  "eid":"relation-a",
  "deco":"locker-block",
  "lock":["key-block-1","key-block-2"]
}]
```

Canonical meaning:

- `deco` = Locker Block.
- `lock` = nonempty array of distinct Key Block IDs.

Legacy scalar form `deco:keyId, lock:lockerId` is inverted on import and saved back canonically without losing unrelated fields. Reject missing, Blind, self, duplicate, multiply-owned, or empty enabled endpoints.

### Hammer / Wall

```json
["hammer-wall", {
  "eid":"relation-b",
  "deco":"wall-pool",
  "hammer":["hammer-block-1"]
}]
```

- `deco` references Blind.
- `hammer` is nonempty array of distinct Block IDs when enabled.

### Pipe — current schema

```json
["pipe", {
  "eid":"pipe-a",
  "deco":"pool-a",
  "m":[0,1,1,1],
  "fr":0.5,
  "s":[3,3,7]
}]
```

`m` is exactly a flat array `[fromX, fromY, toX, toY]` of four finite numbers.

- `eid`: Pipe identity.
- `deco`: owning Blind ID.
- `m`: nonzero axis-aligned local segment.
- `fr`: finite flow rate >0 cells/second.
- `s`: nonempty ordered color array; every color `0..15`.

Import/export additionally validates mouth geometry against target Pool/board/padding and overlap with other enabled Pipes.

Do **not** emit the obsolete `edge/start/width/layers/unit/span` representation described by older documentation.

## 11. Decorator position metadata

### Single-host decorators

Moved Ice, Direction, or Glass badge:

```json
"at": {
  "v": "led-icon-1",
  "x": 0.5,
  "y": 0.5
}
```

Only this tagged version is interpreted. Any legacy/foreign `at` token, such as an array, remains opaque and preserved.

### Linked endpoint positions

Key/Locker and Hammer/Wall endpoint positions use versioned `_led.kp` data keyed by relation ID and endpoint placeable ID. Records are saved in canonical ID order. Unknown future versions are preserved without interpretation.

## 12. Disabled decorator tombstones

Disabled decorators do not appear in runtime-active `entitites`. They are archived as their normal envelopes under:

```json
"_led": {
  "dt": [
    ["ice", {"eid":"ice-a", "deco":"block-a", "count":1}],
    ["pipe", {"eid":"pipe-a", "deco":"pool-a", "m":..., "fr":0.5, "s":[3]}]
  ]
}
```

Tombstones preserve:

- stable decorator ID;
- settings and positions;
- unknown payload fields;
- ability to restore after save/close/reopen.

Deleting the host removes ordinary dependent decorators. Relation endpoint deletion prunes affected endpoints; remove-last disables the relation when restoration semantics apply.

## 13. Import algorithm

Recommended phases:

1. Enforce request size and parse safely.
2. Validate root scalar fields and board dimensions.
3. Decode board mask.
4. Read every entity envelope into ordered raw records; reject duplicate IDs.
5. Parse placeable geometry and Block properties without resolving decorators.
6. Parse Pool canvas descriptors and infer/validate shared pixels/cell.
7. Materialize Pool canvases and validate shape masks.
8. Parse active decorators and `_led.dt` tombstones.
9. Resolve all host/endpoints in a second pass.
10. Validate global occupancy, relation uniqueness, Pipes, boundary geometry, and editor metadata.
11. Construct one canonical candidate and compatibility envelope.
12. Replace session state only after all phases succeed.

## 14. Export algorithm

1. Validate canonical Level and every enabled Pipe.
2. Clone original root compatibility envelope.
3. Patch changed root fields only.
4. Walk original entity order, replacing supported entities by stable ID while preserving their untouched opaque properties and relative order.
5. Append new entities deterministically.
6. Reuse unchanged raw geometry/canvas/row tokens.
7. Re-encode only changed masks, guides, locks, and artwork.
8. Write active decorators into `entitites` and disabled ones into `_led.dt`.
9. Patch `_led.pr` and linked icon metadata only when owned semantics require it.
10. Serialize to a build artifact without changing the session baseline.
11. Persist atomically.
12. Mark artifact accepted and advance baseline only after persistence success.

## 15. Recovery envelope

A session recovery record is separate from runtime JSON and should contain:

```text
recoverySchemaVersion
sourcePath or sourceBinding
sourceFingerprint
sourceTemplateJson
savedBaselineCanonicalOrJson
currentDraftCanonicalOrJson
```

- Validate aggregate size.
- Consume/clear the recovery record before parsing to avoid recovery loops.
- Reject old schema versions rather than maintaining a second hidden legacy parser.
- Restore saved baseline and current draft, but reset Undo/Redo intentionally.
- If source fingerprint/path no longer matches, require explicit recovery/fork decision.
