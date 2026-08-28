# Roadmap

After the shared foundation, Oreak is developed as behavior-complete vertical
slices. A slice is complete only when native Rust and WebAssembly compile,
deterministic tests pass, and the user-visible behavior has an end-to-end path.

## Foundation: History and blame

Status: implemented.

- Validated logical grid and deterministic snapshot hashing.
- Typed map-cell command with no-op suppression.
- Append-only command timeline with duplicate-command protection.
- User-scoped Undo recorded as a compensating event.
- Conflict protection when a later active command touched the same target.
- Cell blame and complete cell-history queries.

## Slice 1: Collaborative map proof

Status: implemented as an in-memory MVP.

- `oreak-protocol` provides typed jsonrpsee methods and subscriptions.
- Axum mounts JSON-RPC as a Tower service at `/rpc`.
- The server owns in-memory timelines with server-ordered sequences.
- Accepted commands and snapshot hashes are broadcast to subscribers.
- The Yew PWA edits dynamically sized maps through the shared core, with
  anchored resize, frame, wheel zoom, and middle-button pan.
- Remote events, local blame, and snapshot resync work.
- RPC calls are session-authenticated, project/level scoped, capability checked,
  and use server-canonical actor/timestamp metadata.
- View-authorized, bounded history pages hydrate activity and exact cell blame
  against the subscription's initial snapshot without missing concurrent events.
- A separate level-presence stream reports connection-scoped rosters,
  authenticated actors, join/leave events, and throttled cell cursors without
  adding ephemeral traffic to the authoritative timeline.

Remaining gate: add an automated two-browser acceptance test for ordering,
hashes, presence cleanup, and Undo conflicts. Map drag samples also need
aggregation into one atomic pointer-up command.

## Slice 2: Projects and durable drafts

Status: partial.

- Implemented in domain/API: personal and organization workspaces,
  project-scoped capability roles, organization administration, Argon2id
  email/password sessions, and project audit records separated from edit
  history.
- Implemented in the browser: session bootstrap, login/registration,
  workspace/project/level catalog, project/level creation and name/duration
  configuration, scoped drafts, and read-only capability gating.
- Remaining: PostgreSQL command persistence, periodic snapshots, email
  verification/recovery, deployment CSRF policy, and IndexedDB recovery for
  unacknowledged commands.

## Slice 3: Placeable domain

Status: core domain and expanded browser parity slice implemented.

- Stable IDs, connected masks, Blocks, Blinds, occupancy, placement, movement,
  rotation, flipping, deletion, decorators, brush strokes, guide barriers, and
  entity provenance are implemented in `oreak-core`.
- The browser renders and hit-tests Block/Blind footprints, displays simplified
  Blind tile colors and entity provenance, and exposes ordered shift-selection,
  atomic grouped drag/keyboard movement, guarded single-entity rotate/flip/delete,
  and placement commands through the authoritative collaboration path and
  offline drafts.
- A shared `8 x 8` Shape Designer validates connected footprints through
  `oreak-core`, creates either Blocks or Blinds, and stores reusable CRUD samples
  in the project-scoped REST catalog.
- Selected-Blind Brush mode exposes previewed, atomic size-1 Paint/Erase
  strokes with interpolated samples and Empty-only flood fill. Strokes remain
  inside the pointer-down guide partition and use the authoritative core/RPC
  path.
- Selected Blocks expose Ice, Direction, and two-click Key/Locker authoring,
  with semantic canvas overlays and Block-only validation in the authoritative
  core. Disabled decorator tombstones, preserved Ice counts, movable badges,
  and batch selection remain incomplete.
- Blind Brush supports a local isolation view without changing command or
  history semantics.
- Remaining: grouped rotate/flip/delete, merge, Blind topology resize, full
  decorator parity, larger brush tips, guide authoring, Divide,
  capacity-normalization workflows, and richer placeable authoring controls.

## Slice 4: Legacy compatibility

Status: adapter implemented; product workflow not integrated.

- Strict loss-preserving LevelData parsing/export and DataCodec behavior have
  golden coverage for Blocks, Blinds, decorators, unknown fields, and codecs.
- Remaining: import/export API and browser workflows, additional Unity golden
  fixtures, and encoding newly authored Blind guide topology.

## Slice 5: Governance, Sandbox, and plugins

Status: domain foundations implemented.

- Reviews, approvals, immutable artifacts, channels, audit/KPI models, exact
  plugin pins, codec certification, a deterministic isolated Sandbox session,
  and the native Lua runner are implemented.
- Remaining: durable orchestration and browser workflows, plugin worker/server
  integration, certified codec execution, Sandbox sand simulation, and full
  release/publish gates.
