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

The working area has a primary left sidebar (Shape Templates, Blame, Project
Explorer, Comments, and Collaboration Session), a secondary right sidebar
(Inspector, Level Structure, and Level Configuration), and a central real-time
canvas. Selecting an entity or cell from the canvas, marquee, or Project Explorer
opens the Inspector tab without overriding a deliberately hidden right panel. Inspector
sections can be collapsed independently; their user-scoped preferences persist in
browser storage. The active sidebar tab expands to show its label; inactive tabs
remain icon-only. Both desktop sidebars have drag handles, one Dock/Float toggle,
and a separate Hide action in their panel controls and the View menu. The active
theme source is shown in the bottom status line, where a user override can be
reset to the project default.

The primary floating tool selection is centered above the canvas and enters
Place Entity, Map Design, Shape Studio, and Image Studio workflows. Context
selection and level zoom controls share a secondary floating toolbox centered
below the canvas, avoiding the selection-blame popover. Context tools appear
only for Place Entity or Map Design: choosing a Sand Block or Pool context
focuses Shape Templates; wall paint, erase, and resize are directly available. Strip Map and Clear All
remain visibly disabled until they have atomic authoritative commands. Shape
Templates is a browse-only library with procedural footprint thumbnails: use its
thumbnail-only grid or named list view, then drag a template using the current
Block/Pool placement context. Shape Studio is the focused modal for saved-shape
creation, editing, and properties. Image Studio imports bounded static PNG, JPEG,
and WebP files as project-scoped,
server-verified shared templates. Users can choose a file or paste a copied
image while Image Studio is open; reads require project view access and imports
require `edit_timeline`. The MVP server holds those templates only for its
running process, so a durable blob store is still required before production. Pixelation
and canvas placement remain unavailable until derived-image and level-asset
commands exist. Comments likewise reserve a shell tab until a comment model and
RPC surface exist.

The central renderer is currently Canvas 2D rather than WebGL; it continues to
render editing and collaboration progress in real time. WebGL and Unity/native
integration are phased in [the UI shell plan](../../docs/LEVEL_EDITOR_UI_SHELL_PLAN.md)
without changing server authority. Right or middle drag pans the canvas, the
wheel zooms around the pointer, and Frame fits the complete level. `Z`, `X`,
and `C` choose the current mode's workspace tools in order; keyboard shortcuts
and the command palette retain access to non-primary modes without restoring a
sidebar mode bar.

Map Resize exposes one draggable handle on each edge. Dragging previews a
single-axis resize, clamps inward movement before it would clip authored cells
or entity footprints, and submits exactly one authoritative `ResizeGrid`
command on pointer-up. Top and left resizes translate content through the core
anchor rules while preserving its screen position for every subscribed client.

## Core placeables

Select mode hit-tests Block and Blind (`Pool` in the browser UI) footprints
before their underlying floor cells. The Inspector tab shows selected entity
ID, kind, origin, shape bounds, and occupied-cell count; Block Capacity is its
own collapsible section with editable capacity layers. A same-type multi-selection
shows common values or literal `<different>` values and can batch-apply a capacity
value where every selected Block has that layer. Pool resolution is configurable
from 1 to 32 pixels per cell through an undoable command with deterministic
nearest-center resampling and can be batch-applied to a same-type Pool selection. The Transform tool moves one grid cell in four
directions, rotates clockwise, flips horizontally, or deletes. Holding and
dragging any entity in Select mode moves it directly after a short movement
threshold, independent of the active Select tool. `Ctrl`+`Shift`-drag duplicates
the selected entity group (decorators are intentionally not cloned); while a
drag is active, `R` rotates the group clockwise around the landing grid point.
`Delete` removes the selected entity group as one undoable operation.
Ctrl/Command or Shift-click adds entities to the ordered selection, and dragging
from an empty cell creates a marquee that selects intersecting footprints.
Workspace arrow keys perform one-cell moves while Transform is active. Commands
remain disabled during initial sync/resync and for users without `edit_timeline`.

Choose the Block or Pool placement context in Shape Studio, then drag a saved
thumbnail from Shape Templates onto an unoccupied floor footprint. One drop emits
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
