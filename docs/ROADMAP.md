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
- The Yew PWA edits an `8 x 8` map through the shared core.
- Remote events, local blame, and snapshot resync work.
- RPC calls are session-authenticated, project/level scoped, capability checked,
  and use server-canonical actor/timestamp metadata.
- View-authorized, bounded history pages hydrate activity and exact cell blame
  against the subscription's initial snapshot without missing concurrent events.

Remaining gate: add collaborator presence and an automated two-browser
acceptance test for ordering, hashes, and Undo conflicts. Map drag samples also
need aggregation into one atomic pointer-up command.

## Slice 2: Projects and durable drafts

Status: partial.

- Implemented in domain/API: personal and organization workspaces,
  project-scoped capability roles, organization administration, Argon2id
  email/password sessions, and project audit records separated from edit
  history.
- Implemented in the browser: session bootstrap, login/registration,
  workspace/project/level catalog, project/level creation, scoped drafts, and
  read-only capability gating.
- Remaining: PostgreSQL command persistence, periodic snapshots, email
  verification/recovery, deployment CSRF policy, and IndexedDB recovery for
  unacknowledged commands.

## Slice 3: Placeable domain

Status: core domain implemented; browser integration partial.

- Stable IDs, connected masks, Blocks, Blinds, occupancy, placement, movement,
  rotation, flipping, deletion, decorators, brush strokes, guide barriers, and
  entity provenance are implemented in `oreak-core`.
- Remaining: complete browser tools and rendering, exact Unity Brush/Divide
  behavior, capacity-normalization workflows, and merge parity.

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
