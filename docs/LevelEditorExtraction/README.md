# Level Editor Extraction Documentation

This folder describes the current Sand Level Editor as an implementation-independent product and data contract. It is intended to guide a new server-backed editor without requiring the reader to understand Unity, C#, editor windows, or the original class structure.

## Scope

The extracted system is a collaborative-capable level-authoring domain with:

- a versioned level document and immutable revisions;
- a logical board of floor/wall cells;
- connected Block and Blind/Pool shapes;
- ordered Block collection layers;
- per-pixel Blind artwork and authoring guides;
- Ice, Direction, Glass, Key/Locker, Hammer/Wall, and Pipe relations;
- map, shape, artwork, distribution, normalization, import, migration, simulation, preview, and test workflows;
- lossless import/export of supported legacy runtime entities and their unknown properties; unsupported entity markers fail import explicitly;
- atomic persistence, stale-write protection, recovery, and batch migration.

The documentation separates three concerns that must not be conflated:

1. **Canonical domain model** — the clean server-side representation and invariants.
2. **Legacy runtime wire format** — the compact JSON required by the existing game.
3. **Editing/session mechanics** — previews, commands, history, temporary simulation, and jobs.

## Source-of-truth note

The extraction is based on the current implementation, including tests and directly used runtime data contracts. Where the existing Level Editor README disagrees with executable behavior, the current source and runtime contracts win.

Two important corrections are already incorporated throughout these documents:

- A current Pipe uses `m`, `fr`, and `s` fields and multiple Pipes may target one Blind. The older `edge/start/width/layers/unit` description is obsolete.
- A Block collect row has an optional fourth maximum-collection-speed value. The effective default radius is resolved from project defaults, with runtime fallback `20`, not a hard-coded `30`.

## Reading order

1. [Product and mechanic specification](<MECHANICS.md>) — modes, commands, previews, tools, simulation, and test flows.
2. [Canonical domain schema](<DOMAIN_SCHEMA.md>) — entities, value objects, IDs, coordinates, invariants, and derived values.
3. [Legacy JSON contract](<LEGACY_JSON.md>) — exact game-facing format, codecs, preservation rules, and compatibility behavior.
4. [Validation and transactions](<VALIDATION_AND_TRANSACTIONS.md>) — validation phases, optimistic concurrency, undo, recovery, atomic saves, and batch jobs.
5. [Server blueprint and API](<SERVER_BLUEPRINT.md>) — service boundaries, resource model, commands, events, jobs, and recommended persistence.
6. [Acceptance specification](<ACCEPTANCE_SPEC.md>) — behavior-level test matrix for a replacement implementation.
7. [Source traceability](<SOURCE_TRACEABILITY.md>) — implementation evidence and coverage map. This is the only intentionally code-oriented document.

## Core terminology

| Term | Meaning |
|---|---|
| Level | The complete authored aggregate: board, placeables, artwork, decorators, timing, and palette reference. |
| Board cell | One logical tile in a bottom-left-origin grid; either Floor or Wall. |
| Shape | A tightly bounded, four-neighbour-connected set of occupied local cells. |
| Block | A movable placeable that owns an ordered collection-layer stack. |
| Blind / Pool | A placeable that owns dense pixel artwork, guide topology, boundary settings, and optional Blind decorators. |
| Collect layer | One ordered Block rule containing color, radius, capacity, lock state, and optional maximum speed. |
| Decorator | An independent, stable-ID behavior or relation attached to a Block/Blind. |
| Tombstone | A disabled decorator preserved outside runtime-active entities so restoring it reuses identity and settings. |
| Guide | An authored white lattice edge that partitions Brush operations but does not affect runtime sand physics. |
| Revision | An immutable canonical document version used for history and stale-write checks. |
| Preview/proposal | A computed candidate bound to a source revision; it cannot apply after its source becomes stale. |
| Sandbox | A private temporary simulation copy with its own history; never a save source. |
| Gameplay preview | Export of the current authored draft into the real runtime; unsaved edits are included. |

## Recommended extraction boundary

The server should own:

- canonical level revisions and validation;
- command execution and deterministic derived calculations;
- import/export adapters;
- preview/proposal tokens;
- migration and batch jobs;
- audit history and optimistic concurrency;
- simulation snapshots when shared or reproducible testing is needed.

Clients should own only presentation concerns:

- panes, viewport projection, hit testing, pointer capture, cursors, temporary visual ghosts;
- local keyboard mapping;
- throttled display previews derived from server-validated candidates;
- rendering of server-provided geometry, diagnostics, and changes.

A client must never be the authority for IDs, validity, capacity arithmetic, import normalization, or legacy-wire preservation.

## Explicit non-goals

- Reproducing Unity window geometry or rendering primitives byte-for-byte.
- Treating the editor Sandbox as exact runtime physics. It is deterministic authoring feedback and has known parity gaps.
- Making the compact legacy JSON the server's primary database schema.
- Automatically normalizing capacity on save, import, boundary change, CAPX import, or gameplay preview.
- Silently fixing invalid geometry, stale previews, unknown fields, or external file changes.
