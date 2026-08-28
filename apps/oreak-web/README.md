# oreak-web

Desktop-first Yew 0.23 PWA shell for the Oreak technical editor. The map and
core-placeable editor uses `oreak-core` locally and the generated `OreakRpcClient` from
`oreak-protocol` for authoritative snapshots, commands, undo, and level-event
subscriptions. A separate connection-scoped presence subscription carries the
level roster and ephemeral cell cursors over the same WebSocket. Draft snapshots
and theme overrides are stored in browser local storage as a startup cache, but
timeline mutation requires a synchronized live RPC session.

From the workspace root:

```text
cargo run -p bootstrap -- init
cargo run -p bootstrap -- dev
cargo run -p bootstrap -- build --release
```

Cargo's `--` separator is required before bootstrap command arguments. To run
only this frontend against an existing backend:

```text
cargo run -p bootstrap -- frontend --backend 127.0.0.1:3000 --open
```

The app bootstraps the HttpOnly session, presents login/registration when
needed, and presents one searchable workspace/project/level hierarchy before
opening the editor. Registered users can be invited to projects by email and
must explicitly accept from their catalog. Each project configuration dialog
exposes its default theme and, for authorized users, its member roles and
pending invitations. The RPC endpoint is `ws(s)://<current-host>/rpc`; serving the
compiled PWA from `oreak-server` keeps the editor and WebSocket on the same
origin. The bootstrap frontend command generates a Trunk 0.21.14 configuration
under `target/bootstrap` with HTTP and WebSocket proxies. Both proxies override
the forwarded `Host` with the browser-facing authority so the server's
same-origin RPC check remains valid.

The client subscribes first, anchors bounded paginated history to the initial
subscription snapshot, and buffers later events until hydration completes.
Activity and cell blame therefore include prior server events and exact Undo
provenance. Hydration currently has a 4,096-event safety cap. Presence is
connection-scoped so multiple tabs retain independent cursors, while the UI also
reports distinct authenticated actor counts and join/leave notices. Cursor
updates are cell-based, deduplicated, and throttled. Draft snapshots are scoped
by authenticated user, project, and level. Project capabilities make the editor
read-only unless the exact `edit_timeline` capability is present.
If the subscription closes or hydration fails, all timeline controls become
read-only and reconnect automatically with bounded exponential backoff. The
client never falls back to an independent offline edit timeline.

The working area keeps mode and action selection separate. Select, Map, Brush,
and Sandbox modes are tabbed across the top of the canvas. The bottom tool tabs
show only actions for the current mode; `Z`, `X`, and `C` choose those tools in
order. The left sidebar switches between the selected tool's behavior and
reusable entity templates. Both desktop sidebars have drag handles and can be
docked, floated over the canvas, or hidden from their panel controls and the
View menu. The right sidebar switches between Inspector, Activity, and Blame
without mixing those workflows into one scrolling panel. The rectangular canvas
fills the remaining workbench; right or middle drag pans it, the wheel zooms
around the pointer, and Frame fits the complete level.

Map Resize exposes one draggable handle on each edge. Dragging previews a
single-axis resize, clamps inward movement before it would clip authored cells
or entity footprints, and submits exactly one authoritative `ResizeGrid`
command on pointer-up. Top and left resizes translate content through the core
anchor rules while preserving its screen position for every subscribed client.

## Core placeables

Select mode hit-tests Block and Blind (`Pool` in the browser UI) footprints
before their underlying floor cells. The Inspector tab shows the selected
entity ID, kind, origin, shape bounds and occupied-cell count, plus Block
collect-layer or Pool tile summaries. Pool resolution is configurable from 1 to
32 pixels per cell through an undoable command with deterministic nearest-center
resampling. The Transform tool moves one grid cell in
four directions, rotates clockwise, flips horizontally, or deletes. Holding and
dragging any entity in Select mode moves it directly after a short movement
threshold, independent of the active Select tool. Ctrl/Command or Shift-click
adds entities to the ordered selection, and dragging from an empty cell creates
a marquee that selects intersecting footprints. Workspace arrow keys perform
one-cell moves while Transform is active. Commands remain disabled during
initial sync/resync and for users without `edit_timeline`.

Drag a Block or Pool template from the Place panel, or a connected draft from
Entity templates, and drop it on an unoccupied floor footprint. One drop emits
one authoritative placement command for the complete shape. The default Block
mirrors the reference editor: color index `1`, default
collect radius, unlimited capacity, and unlocked capacity. The default Blind is
a blank `32 x 32`-pixel tile at `32` pixels per cell with no guides. Entity IDs
are generated from the browser's random command-session prefix plus a monotonic
counter; deterministic IDs and platform randomness remain outside
`oreak-core`. The server still validates placement and owns command ordering.

The canvas renders Blocks separately from walls and renders Pools with a
distinct footprint, empty-canvas hatch, and simplified color-index pixels. A
multi-cell Block or Pool is one entity and renders as one continuous fill with
outlines only on exposed shape edges; adjacent independent entity IDs remain
separate.
Activity, selected-entity history, and entity blame are hydrated from the same
exact event stream as cell history.

## Blind brush

Select a Pool in Select mode, then enter Brush mode with `B`. Choose Paint or
Fill from the bottom tool tabs; `B` and `F` switch those tools from the keyboard.
LMB paints with the active color, `Shift+LMB` erases, and Fill applies Empty-only
four-neighbor flood fill. Right or middle drag remains viewport pan in Brush
mode. Number keys `1` through `9` select matching color
indices and `0` selects color `10`. Pointer samples are interpolated with the
size-1 tip, clipped to the Pool footprint and pointer-down guide partition,
previewed locally, and sent
as one undoable command on pointer-up. Escape, pointer cancellation, mode/tool
changes, and collaboration snapshot replacement discard the preview.

## Decorators

Select a Block in Select mode to edit its decorators. `I` toggles Ice, the
inspector sets its non-negative blocking count, and `D` cycles Direction through
None, Horizontal, and Vertical. `K` starts or cancels Key/Locker assignment;
while active, click a different Block that is not already another Key's Locker.
Reassignment preserves the active relationship ID. Ice rims/counts, Direction
badges, Key/Locker badges, links, and Locker rims render directly on the canvas.
All edits use the same authoritative command and Undo path as entity edits.

This slice intentionally excludes merge, grouped rotate/flip/delete, decorator
badge positioning and disabled tombstone parity, capacity normalization, larger
Square/Round/Diamond tips, guide authoring, Divide, Block color sampling, and
browser Sandbox commands.
