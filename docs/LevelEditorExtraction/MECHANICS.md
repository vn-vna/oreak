# Product and Mechanic Specification

This document specifies observable behavior. A replacement client may use different layouts and input bindings, but commands, state transitions, validation, and commit semantics should remain equivalent.

## 1. Universal interaction contract

High-impact interactions use the same transaction pattern:

1. Capture an immutable source revision and interaction baseline.
2. Build live candidates without mutating the canonical document.
3. Display validity and diagnostics continuously.
4. On confirmation/release, revalidate against the captured source revision.
5. Commit one atomic command if the candidate is valid and changed.
6. Otherwise restore the exact baseline and create no history entry.

This pattern applies to move, duplicate, rotate-with-drag, map/Blind resize, wall strokes, Brush strokes, Fill, Line, Divide, CAPX placement, distribution, normalization, picture-grid migration, Sand Studio Apply, and batch migration.

Recommended server representation:

```text
proposalToken = hash(levelId, sourceRevision, commandType, normalizedInputs,
                     projectDefaultsFingerprint, externalSourceFingerprint)
```

An Apply request with a stale token must fail, never silently rebase.

### Cancellation

- Escape cancels the innermost active interaction first.
- Right mouse commonly cancels an active left-button gesture, but may have a mode-specific primary action when no gesture is active.
- Mode/document changes, focus loss, pointer-capture loss, or changed source revision cancel transient candidates.
- Cancel and invalid/no-op completion never enter history.

### History

- Authored history is bounded to 80 immutable snapshots/commands.
- New commands clear redo.
- Undo/Redo is unavailable while a gesture, splitter, or shape-library interaction owns input.
- Sandbox has a completely separate history.
- Shape-library operations are outside Level history.

## 2. Workspace states

The five main workspaces are:

| Workspace | Purpose |
|---|---|
| Select | Inspect, select, place, transform, merge, edit properties and relations. |
| Distribution | Allocate effective selected sand across selected Block rows by weights. |
| Map | Edit Floor/Wall cells and board bounds. |
| Brush | Edit Blind pixel artwork, Fill regions, guides, Divide plans, and CAPX placement. |
| Sandbox | Run a temporary deterministic authoring simulation. |

Real Test and Gameplay PLAY are launch workflows rather than persistent document modes.

Mode changes publish valid delayed property edits first, cancel incompatible transients, and preserve viewport pan/zoom unless explicitly reframed. Entering Brush clears object selection and starts Paint when entered through a true mode change. Clicking the already-active mode tab is a no-op; invoking the active mode through a shortcut/programmatic transition may act as safe cancel and reset Brush to Paint.

## 3. Viewport and inspection

- Logical geometry is Y-up.
- Wheel zoom is anchored at the pointer.
- Middle-drag or Alt+left-drag pans.
- Home frames the whole map.
- A non-authored subcell guide may appear at medium/high zoom; it is never saved.
- Hold Shift for capacity information. Over empty space, label all placeables; over one object, narrow to that object. Shift+pointer must not also edit.
- Exactly one selected Blind owns the local pixel ruler. Hover fallback is allowed only with no selection.
- Ruler coordinates include lattice vertices from `0` through canvas width/height and adapt tick spacing to zoom.
- At high zoom Blind art renders exact pixel runs; below roughly `0.8` screen pixels per source pixel, render a coverage-weighted per-cell mip. Sparse art must remain visibly sparse.

## 4. Select workspace

### 4.1 Selection

- Plain click selects one placeable as primary.
- Ctrl/Cmd-click toggles stable multi-selection membership.
- Clicking an already-selected member without modifier preserves the group so dragging moves the group.
- Plain Floor click clears object selection; Wall remains inspectable.
- Ctrl/Cmd+A selects every Block and Blind when no text/gesture owns input.
- Empty-space drag begins a rectangle selection after a small movement threshold. Any placeable with an occupied cell intersecting the rectangle is selected; Ctrl/Cmd adds.
- Right-down on a placeable selects it. Right-down on a decorator badge selects its host and begins badge-position drag.

### 4.2 Move and nudge

- Drag captures the exact grabbed tile and moves the selected group by one common cell delta.
- A small pointer threshold distinguishes click from drag.
- Shift locks to the dominant axis.
- Arrow keys nudge the complete selection by one cell.
- Candidate must remain inside board, on Floor, mutually non-overlapping, and disjoint from unselected placeables.
- Invalid release restores all original positions.

During a move, rotation and Block recolor commands may update the same candidate. A valid release commits movement, orientations, and colors as one command; invalid release restores all of them.

### 4.3 Duplicate drag

Ctrl/Cmd+Shift-drag:

- leaves originals untouched;
- moves translucent copies;
- attempts every copy at release;
- commits all individually valid copies as one command;
- discards invalid copies;
- selects the copies that were created.

A modern replacement should preferably commit on pointer release only. The old implementation can also commit when a duplicate modifier is released, which is nonstandard and should not be copied unless compatibility demands it.

### 4.4 Transform

- Rotate: every selected placeable rotates clockwise around its own lower-left tight-bound origin.
- Horizontal flip: every selected placeable mirrors inside its own tight bounds.
- Group validation is atomic; one invalid member rejects the whole command.
- Blind artwork and guides transform with its shape.
- Stable IDs, collect stacks, boundary settings, and decorator relations survive.
- Symmetric/no-op transforms create no history.

### 4.5 Merge

Only homogeneous Block-only or Blind-only selections can merge.

Common rules:

- World-cell union must be four-neighbour connected.
- Primary/last-selected ID and compatible metadata survive.
- Other selected placeables are removed in the same transaction.
- Result must pass size, board, wall, and overlap validation.

Block merge:

- Result must fit within `8 x 8`.
- Identical complete ordered collect stacks merge directly.
- Differing stacks require explicit selection of one complete survivor.
- Rows, radii, capacities, max speeds, and locks are copied together; never concatenate, sort, or sum implicitly.

Blind merge:

- Tight bounding area must be <=64.
- Artwork remaps by world pixel without flipping/resampling.
- Existing guide sets are unioned and deduplicated.
- Shared edges between cells that belonged to different source Blinds become guides.
- Outer boundaries and edges internal to one original Blind are not added merely because of merge.
- Boundary configurations that cannot be reconciled must be made consistent first.

### 4.6 Delete

Delete/Backspace removes the complete selected set in one command. Dependent decorators/endpoints are removed, pruned, or tombstoned according to relation rules. Unrelated entities are unchanged.

### 4.7 Direct footprint Grid Edit

With exactly one placeable:

- Add an adjacent cell or remove an enabled cell.
- Bounds automatically tighten/expand.
- Reject disconnection, empty result, wall/overlap/map exit, size violation, invalid attached Pipes, and removal of Blind cells containing painted sand.
- Save current footprint as a reusable Shape sample through an explicit command.

## 5. Shape Designer and library

- Designer canvas is `8 x 8`.
- Toggle cells, name the Shape, validate nonempty four-connectivity, then Save/Update.
- Drag a saved Shape onto the board: primary drag creates Block; alternate/right drag creates Blind.
- The grabbed occupied tile stays under the pointer.
- New Blind canvas uses current level pixels/cell and begins Empty.
- Library rename/delete/reorder changes the project library only.
- Existing placed instances retain their copied shape/name.
- Duplicate/malformed library records disable unsafe reorder rather than writing filtered indices back to the wrong record.

## 6. Map workspace

### 6.1 Cell strokes

- Left stroke turns cells into Floor.
- Right stroke or Delete turns cells into Wall.
- Sparse pointer samples are interpolated.
- Leaving/re-entering the board breaks interpolation.
- A cell beneath a placeable cannot become Wall.
- Entire stroke commits once on release.

### 6.2 Board resize

- Hover edges/corners for resize handles.
- Resize in whole cells with opposite edge/corner anchored.
- Remap retained board cells and all placeable origins by the common shift.
- Reject dimensions outside limits and any result that crops a placeable.

For a new server/client, use the universal invalid-release rule: if the final requested candidate is invalid, restore baseline. Do not reproduce the old inconsistency in which Map resize may commit its last valid intermediate while showing a later invalid requested size.

## 7. Blind edge resize

With exactly one selected Blind:

- Drag one of four tight bounding rails.
- Opposite world edge stays fixed.
- Growth extrudes the occupied pattern on the dragged edge.
- Shrink crops strips from that edge.
- Irregular footprints are not silently filled to rectangles.
- Retained artwork stays at the same world-pixel position.
- Cropped pixels disappear; new pixels start Empty.
- Guides and Pipe mouths remap with the geometry.
- Reject empty/disconnected result, bounding area >64, board/wall/overlap conflict, or invalid attached Pipe.
- Invalid final release restores baseline, not a prior intermediate.

## 8. Block collect-layer editing

For one selected Block, expose the complete ordered inner-to-outer stack.

Commands:

- add, remove, disable/enable, reorder;
- set color `0..15` or disabled `-1`;
- set/unset radius;
- set/unset finite/Unlimited capacity;
- set/unset maximum collect speed;
- lock/unlock capacity independently per row.

Rules:

- Locked row capacity is read-only until unlocked.
- Removing a locked row is rejected.
- Row order is gameplay state.
- Number shortcuts recolor the current inner active row only.
- Move/transform/duplicate/merge survivor copy/Undo/Redo preserve full rows and locks.

## 9. Decorator authoring

### Ice / Direction / Glass

- Toggle Ice on selected Blocks.
- Cycle a uniform Block selection: no Direction -> Horizontal -> Vertical -> none. Mixed states first normalize to Horizontal.
- Toggle Glass on selected Blinds.
- Count editors commit only valid non-negative integer input and revalidate decorator identity.
- Count zero disables counted decorator while preserving tombstone identity/settings.

### Key / Locker

- Exactly one selected Block becomes Locker.
- Subsequent plain Block clicks toggle Keys while assignment remains active.
- Reject self, Blind, and a Key already owned by another enabled relation.
- Each click is one history command.
- Escape/reinvoke/change context ends linking without another command.

### Hammer / Wall

- Exactly one selected Blind becomes Wall.
- Subsequent Block clicks toggle Hammers.
- Removing last Hammer disables the relation as tombstone.
- Current editor Sandbox does not simulate this mechanic.

### Badge positions

- Right-drag an endpoint badge.
- Position is continuous in host-local coordinates.
- Whole badge must remain on occupied host footprint.
- Invalid release restores prior position.
- Positions persist across toggle/save/reopen and fall back per endpoint after invalidating shape edits.

### Pipe

Current behavior:

- One Blind may own multiple Pipes.
- Add a one-cell or two-cell mouth, or draw a custom mouth.
- Select among Pipes, enable/disable, edit `from`/`to`, flow rate, and ordered color stacks.
- Drag either endpoint to resize; drag the body along its edge.
- Endpoint coordinates snap to current Blind-pixel precision.
- Mouth must remain a continuous exterior footprint boundary and must have playable inlet lanes.
- Enabled mouths cannot overlap on one Blind.
- Disable retains configuration for restore.
- Pipe reserve participates in authored capacity balance.
- Use gameplay preview for flow testing; the editor Sandbox currently does not emit Pipes.

## 10. Brush workspace

The level has one shared pixels/cell value. Brush operations always target one press-time Blind partition.

### Paint / Erase

- Tips: Square, Round, Diamond.
- Size: `1..64` pixels.
- Left paints current color; right erases.
- Interpolate between pointer samples.
- Clip to one captured guide/shape partition.
- Blind bounds, holes, and guides are barriers; existing colors are not.
- Leaving the region or changing tip/size/color breaks interpolation.
- Pointer release commits the complete stroke once.

### Fill

- Starts only on an Empty paintable pixel.
- Fills the four-neighbour Empty region within the captured partition.
- Existing painted pixels, guides, holes, and bounds are barriers.
- Preview on press; commit on release.

### Free guide Line

- Start on a Blind boundary/hole boundary or existing guide vertex.
- Snap to nearby lattice vertices in screen space.
- Endpoint must be supported in the same Blind.
- Generate deterministic axis-aligned unit-edge staircase without leaving the footprint.
- Guides change geometry only, not color.

### Divide

A Divide is an explicit proposal:

```text
weights: 2..16 positive integers, normalized by greatest common divisor
metric: PaintablePixels | Span
direction: Auto | Vertical | Horizontal
```

The existing UI exposes at most 8 parts even though the semantic model supports 16; a server should support 16 and let clients impose a presentation limit only if intentional.

Planner preference:

1. clean axis bands;
2. exact axis-major serpentine paths;
3. connected spanning-tree cuts for pixel-count metric;
4. re-flood-fill every candidate before acceptance.

Apply inserts all generated guides as one command. Stale/empty/invalid proposals cannot apply.

### Eyedropper and color selection

- Eyedropper from a Block copies its active color.
- Numeric shortcuts choose common colors.
- A temporary radial palette may expose C1..C15.
- Unsupported/missing palette slot produces diagnosis, not remapping.

## 11. CAPX artwork import

### Source validation

- `.capx` path, <=32 MiB.
- Width/height `1..4096`; total <=4,194,304 pixels.
- Palette `1..16`; finite normalized RGBA; exact pixel count and valid indices.
- Source uses bottom-left coordinates and is immutable.

### Mapping and approval

- Map to available gameplay Blind colors C1..C15 only.
- Use deterministic CIEDE2000; ties choose lower slot.
- Do not compact or substitute unavailable slots.
- Palette index 0 is an ordinary source color unless explicitly marked Empty.
- Alpha 0 always skips; alpha below configured cutoff skips; no alpha blending.
- Color Review reports source-to-profile mapping, changed/merged/skipped counts, and Delta-E.
- Any source, transparency, or profile change invalidates approval.

### Placement

Fit modes:

- Custom;
- Contain;
- Cover;
- Stretch;
- Fit inside actual partition shape, including holes/concavity.

Write modes:

- Empty Only: preserve existing painted destination pixels.
- Replace: repaint only where source samples are non-transparent. Skipped source never erases.

Rules:

- Always nearest-neighbour sample from the original mapped source, never a previous preview.
- Destination dimensions `1..4096`, total <=4,194,304.
- Fit-inside search must return an exact supported fit or an error, never a near-fit.
- Clip to captured partition and paintable shape.
- Report clipped, written, changed, preserved, and resulting per-color totals.
- CAPX Apply changes Blind artwork only, creates one history entry, and does not normalize Block capacities.

## 12. Sand Studio

Sand Studio edits one Blind in an isolated private canvas session.

Tools:

- brush and eraser;
- custom validated brush mask with integer scale;
- connected-color Fill;
- profile-indexed Gradient;
- color picker;
- rectangular selection, move, nudge, clear.

Behavior:

- Operations respect paintable footprint.
- Fill is four-neighbour and color-bounded.
- Gradient recomputes from press-time baseline by projection onto drag vector and nearest profile color.
- Selection move is atomic and clears source before copying baseline values to a valid destination.
- Local history is capped at 64 and is independent from Level history.
- Apply is rejected if the source Level revision changed.
- Successful Apply replaces only that Blind canvas and creates one Level history entry.

## 13. Pixels-per-cell migration

### Current-level migration

- Preview from exact source revision/defaults.
- Resample artwork nearest-neighbour inside each logical cell.
- Preserve holes/guides and unrelated data.
- Compute actual painted count per color before/after.
- Scale finite Block total per color by actual count ratio with deterministic integer rounding.
- Hold locked finite rows fixed.
- Preserve disabled, zero, and Unlimited semantics.
- Distribute residual proportionally among adjustable positive rows using deterministic largest remainders.

Reject atomically on color loss, capacity without source art, integer overflow, locked subtotal above target, all-locked mismatch, positive-row collapse, insufficient flexible range, or memory limit.

### Batch migration

- Requires catalog snapshot and clean current authored level.
- Deduplicate aliases by canonical path.
- Global errors block the batch; per-file errors skip those files before transaction.
- Eligible files are one all-or-rollback transaction.
- Revalidate catalog/default/source fingerprints before write.
- Back up changed files, verify backups, write/flush/hash-verify results.
- Roll back touched files in reverse order on failure.
- Skipped files are never opened for write or backup.

## 14. Distribution and capacity normalization

### Distribution

- Target selected Blocks and selected Pools/guide groups.
- Pool selection includes all its groups; group selection may be toggled separately.
- Effective playable sand, not excluded paint, is the source quantity.
- Apply weights by color to eligible unlocked Block rows.
- Zero weight for nonempty selected target is invalid.
- Apply is one atomic command.

### Capacity diagnostics

For every color, derive:

```text
sandTarget = playablePaintedBlindPixels + activePipeReserve
blockCapacity = finite enabled Block capacities (plus Unlimited state)
delta = sandTarget - blockCapacity
```

Equal global totals do not imply per-color balance.

### Normalize

Normalize is always preview/confirm and never automatic.

- Sand targets are source of truth.
- Change only enabled collect-row capacities.
- Preserve locked rows by default.
- Convert adjustable Unlimited rows to finite if needed.
- For repeated same-color finite rows, remove surplus from largest first; add deficit to largest headroom first; snapshot order breaks ties.
- For adjustable Unlimited rows, preserve finite subtotal when possible, split residual evenly, and assign remainder in snapshot order.
- Block-only color targets zero.
- Sand/Pipe color with no same-color Block is structural conflict.
- Locked Unlimited, locked subtotal > target, or insufficient unlocked range is conflict.
- Optional explicit “allow locked changes” computes a separate proposal and unlocks only rows whose values must change.
- No partial apply.

## 15. Sandbox

### Isolation and lifecycle

- Enter only after pool-boundary play validation.
- Deep-copy authored Level into private state.
- Resolve/freeze playable pool geometry in the copy and clear nonplayable pixels there only.
- Keep separate counters, selection, history, scheduler.
- Leave discards everything.
- Restart creates a fresh copy from the latest authored revision/defaults.
- File commands are disabled; authored dirty state/history never change.

### Block movement

- Only Blocks move; Blinds/Pipes can be inspected.
- Arrow/button moves are one-cell cardinal commands.
- Analog drag preserves grab offset and advances hook cell-by-cell.
- Prevent tunneling through walls/placeables.
- Diagonal requires both orthogonal intermediate placements; otherwise slide on free/dominant axis.
- Ice count >0 blocks movement.
- Direction limits movement axis.
- Key/Locker does not block movement.
- Sandbox `Delete` removes the selected Block from the temporary copy and applies the same global Ice decrement as completion; `Backspace` intentionally does not.

### Sand tick

Target frequency is 30 deterministic ticks/second, with at most one full tick per update.

Order:

1. Move free grains one pixel downward within each Blind.
2. Determine each Block's active collect row.
3. Resolve Blocks in stable ID order.
4. Resolve candidate contacts in deterministic Blind/pixel/direction order.
5. For each orthogonal shared-edge subpixel lane, scan inward through Empty up to active radius.
6. First painted grain owns the lane: collect if color matches, otherwise block grains behind it.
7. Claim each grain at most once.
8. Apply removals against the same post-gravity frame.
9. Compact each affected column segment once, preserving remaining color order.
10. Update per-row counters, promote completed finite row for the next tick, or remove Block after its final finite row.
11. For each removed Block, decrement every surviving enabled Ice once; clamp at zero and tombstone when exhausted.

Additional rules:

- Guides are ignored by gravity/contact.
- Corner-only contact does not collect.
- Unlimited active row never completes.
- Newly promoted row waits until next tick.
- Key/Locker gates collection while any Key remains; final-Key removal unlocks on the next tick.
- Current Sandbox does not apply max collection speed, Glass, Hammer/Wall, or Pipe emission. Treat it as approximate authoring feedback, not exact runtime physics.

### Sandbox history

- Manual changed step: one history entry.
- Idle step: no entry.
- Contiguous automatic changed run: coalesced into one entry.
- Undo/Redo restores snapshot and counters together and pauses automatic ticking.

## 16. Gameplay PLAY and Real Test

### Gameplay PLAY

- Export the current authored draft, including unsaved edits and pending valid property commit.
- Never export Sandbox temporary state.
- Validate pool policy and full JSON before launch.
- Do not save, normalize, or change progression/catalog mappings.
- Install a temporary runtime level-0 override, launch configured gameplay, and restore prior override/play settings on stop/cancel.

### Real Test

- Export and runtime-verify current draft.
- Launch the dedicated simulator scene with the draft directly.
- Freeze editor controls behind an input-consuming overlay.
- Preserve draft, saved baseline, history, selection, tools, viewport, and paused Sandbox session.
- Restore start-scene and play-mode settings whether exit is requested in-editor or Play Mode is stopped externally.
- Invalid payload must not arm the test.
