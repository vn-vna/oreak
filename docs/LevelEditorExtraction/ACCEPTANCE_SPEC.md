# Acceptance Specification

A replacement is complete only when these behavior-level contracts pass. Tests should operate through public domain/API boundaries and avoid dependencies on the original UI technology.

## 1. Canonical schema

- [ ] New Level is 8x8 Floor, 90 seconds, configured/default 32 pixels/cell, empty entities.
- [ ] Board dimensions enforce 1..256 and <=65,536 cells.
- [ ] Board and pixel indexing are bottom-left row-major.
- [ ] Shape accepts 1x1, deduplicates repeated coordinates, tight-normalizes, and rejects empty or disconnected normalized input.
- [ ] Block rejects bounds >8x8; Blind accepts elongated area<=64 and rejects area>64.
- [ ] Placeable rejects out-of-board, Wall, and overlap.
- [ ] IDs are globally unique across placeables and decorators.
- [ ] Canonical snapshots are immutable or externally behave as immutable.

## 2. Block layers

- [ ] Ordered rows preserve exact order through every unrelated command.
- [ ] Disabled=-1, colors 0..15, null defaults, Unlimited=-1, zero-capacity skip work.
- [ ] Optional fourth max-speed value is finite and round-trips.
- [ ] Active row is first enabled nonzero resolved row.
- [ ] Unlimited active row blocks later promotion.
- [ ] Lock is independent per row and rejects locked capacity change/removal.
- [ ] Project-default changes affect null fields but not explicit values.

## 3. Blind canvas and guides

- [ ] Canvas dimensions equal shape bounds * pixels/cell.
- [ ] Painted hole pixels reject.
- [ ] Resolution migration does nearest-neighbour inside each logical cell with no cross-cell bleed.
- [ ] Move/rotate/flip/resize/merge preserve or transform artwork as specified.
- [ ] Guides partition Brush operations but do not change colors, capacity connectivity, or simulation.
- [ ] Exact and coverage-mip rendering preserve sparse coverage semantics.

## 4. Selection and transforms

- [ ] Stable Ctrl/Cmd multi-selection and marquee intersection.
- [ ] Group move uses common delta and is atomic.
- [ ] Invalid move release restores exact baseline.
- [ ] Rotate and flip transform each member in its own tight bounds and reject group conflicts atomically.
- [ ] Duplicate creates valid subset in one revision and reports rejected copies.
- [ ] Delete updates dependent decorators/relations atomically.

## 5. Merge and resize

- [ ] Block merge requires connected result <=8x8 and explicit whole-stack survivor when stacks differ.
- [ ] Blind merge keeps primary ID, remaps art without resampling, and adds only cross-source internal guides.
- [ ] Blind edge growth extrudes shape pattern and starts new pixels Empty.
- [ ] Blind shrink crops art and preserves retained world-pixel alignment.
- [ ] Board resize anchors opposite edge and rejects cropping placeables.
- [ ] All invalid final releases restore baseline; no last-valid surprise commit.

## 6. Decorators

- [ ] Host-kind matrix enforced.
- [ ] At most one non-Pipe kind per host; multiple Pipes allowed.
- [ ] Count zero tombstones Ice/Glass and re-enable restores default one.
- [ ] Direction cycle and numeric/string wire compatibility.
- [ ] Key/Locker endpoint role, uniqueness, self, removal, chain, and last-Key tombstone rules.
- [ ] Hammer/Wall role and endpoint rules.
- [ ] Badge positions survive disable/re-enable and save/reopen; invalidated endpoint falls back independently.
- [ ] Pipe mouth axis/boundary/gravity/playable-lane/overlap rules.
- [ ] Pipe schema is `eid/deco/m/fr/s`; `m` accepts/emits exactly flat `[fromX,fromY,toX,toY]`; obsolete schema is not emitted.

## 7. Brush and Divide

- [ ] Tip footprints and continuous interpolation for Square/Round/Diamond.
- [ ] One press-time partition clips entire stroke.
- [ ] Leaving/re-entering does not bridge barriers.
- [ ] Fill is four-neighbour Empty-only and barrier-aware.
- [ ] Line anchors and deterministic path remain inside footprint.
- [ ] Divide normalizes weight GCD, supports semantic 2..16 parts, and returns deterministic valid partitions.
- [ ] Stale Divide cannot apply.
- [ ] Every successful gesture/proposal creates exactly one authored revision.

## 8. Pool boundary and capacity

- [ ] Null inherits; explicit zero overrides project defaults.
- [ ] Reports painted/playable/excluded by color and reason without erasing art.
- [ ] Valid-with-loss save vs adopted-play rejection vs invalid-geometry failure remain distinct.
- [ ] Capacity target includes playable Blind pixels and active Pipe reserve.
- [ ] Normalize preserves locks by default and exposes conflicts.
- [ ] Allow-locked plan unlocks only entries that actually change.
- [ ] Same-color deterministic surplus/deficit tie behavior.
- [ ] No matching Block color is structural conflict; no partial apply.
- [ ] Distribution uses selected effective groups, validates weights, and preserves locks.

## 9. CAPX

- [ ] Validate extension, 32 MiB, dimensions/pixel count, palette, RGBA, indices.
- [ ] Lower-left orientation preserved.
- [ ] CIEDE2000 reference pairs and deterministic lower-slot ties.
- [ ] No unavailable-slot compaction/substitution.
- [ ] Palette zero and alpha cutoff semantics.
- [ ] Mapping requires explicit review approval and fingerprints source/profile/options.
- [ ] Contain/Cover/Stretch/InsideShape/Custom behavior.
- [ ] Every resize samples original mapped source.
- [ ] EmptyOnly and Replace never erase skipped/transparent samples.
- [ ] Stale/invalid/no-op placement cannot apply.
- [ ] Apply changes one canvas only and does not normalize capacity.

## 10. Sand Studio

- [ ] Brush, custom brush, Fill, color pick, gradient, and selection move honor footprint.
- [ ] Gradient recomputes from press-time baseline.
- [ ] Selection move is atomic, clears source, and rejects outside destination.
- [ ] Local history max 64, one entry per continuous gesture, redo cleared on new edit.
- [ ] Stale Level rejects Apply.
- [ ] Apply creates one Level revision and preserves all non-target data.

## 11. Pixels-per-cell migration

- [ ] Range 1..128 and same-value no-op.
- [ ] Canvas/guide resampling and per-level memory limits.
- [ ] Capacity uses actual per-color count ratio and deterministic integer allocation.
- [ ] Locked, disabled, zero, and Unlimited semantics preserved.
- [ ] Every blocker leaves source unchanged.
- [ ] Current-level Apply is one revision.
- [ ] Batch aliases deduplicate by canonical source.
- [ ] Global blockers vs per-file skips behave distinctly.
- [ ] Eligible files are all-or-rollback with verified backups/hashes.
- [ ] Skipped files are not opened for write or backup.

## 12. Sandbox

- [ ] Deep copy and authored isolation.
- [ ] Boundary projection changes Sandbox copy only.
- [ ] Cardinal/analog motion and no-tunneling/corner rules.
- [ ] Ice, Direction, Key/Locker movement/collection behavior.
- [ ] Gravity one pixel before collection.
- [ ] Orthogonal contact only; mismatch shields grains behind.
- [ ] One nearest matching grain per lane per tick and stable ownership order.
- [ ] Compaction preserves color order and ignores guides.
- [ ] Sequential rows, promotion waits next tick, Unlimited never completes.
- [ ] Final Block removal and global Ice decrement.
- [ ] Manual step history, auto-run coalescing, idle no-op, Undo pauses.
- [ ] Restart from latest authored document; leaving discards all.
- [ ] Known non-parity is documented: no max-speed, Pipe, Glass, Hammer/Wall simulation.

## 13. Gameplay and Real Test

- [ ] PLAY exports unsaved authored draft and never Sandbox state.
- [ ] Pending valid property commit is included.
- [ ] Invalid export does not launch.
- [ ] Export does not save/normalize/change history/baseline.
- [ ] Temporary runtime override/settings are restored on cancel/stop.
- [ ] Real Test verifies before switching scene.
- [ ] Editor input/history is blocked while test is active.
- [ ] Exit and external stop restore editor state and prior play settings.

## 14. Legacy JSON

- [ ] Root typo `entitites` preserved.
- [ ] Recognized passthrough root fields `dif`, `v`, and `desc` round-trip unchanged.
- [ ] Pool runtime fields `gdx`, `gdy`, `gps`, `mfs`, `sfr`, `col`, `lmin`, and `lmax` round-trip; resolved gravity participates in Pipe validation.
- [ ] Unknown properties on supported records are preserved, while unsupported entity markers reject import.
- [ ] Duplicate properties/trailing data/depth violations reject.
- [ ] Two-pass entity reference resolution independent of order.
- [ ] `bdat` boolean mapping and `g.r/g.d` geometry.
- [ ] DataCodec checksum/compression/length validation.
- [ ] Unknown root/entity fields and unchanged raw tokens preserved.
- [ ] `cc` four-value current contract and per-row lock suffix.
- [ ] Blind top-down wire to bottom-up canonical Y conversion.
- [ ] Placeholder `1x1/null` does not infer resolution.
- [ ] `_led.pr` agreement and no-Blind preservation.
- [ ] Active decorators vs `_led.dt` tombstones.
- [ ] Tagged single-icon and linked endpoint position metadata.
- [ ] Build -> reopen -> semantic equality on every supported fixture.

## 15. Save, recovery, concurrency

- [ ] Complete candidate parse before state replacement.
- [ ] If-Match stale revision rejection.
- [ ] Idempotency key repeat returns same result.
- [ ] Invalid/no-op/cancel creates no revision.
- [ ] External source fingerprint conflict blocks ordinary Save.
- [ ] Temporary write + flush/verify + atomic replace.
- [ ] Baseline advances only after persistence success.
- [ ] Recovery validates version/size/fingerprint, restores draft+baseline, resets history.
- [ ] Corrupt recovery cannot loop.
- [ ] Proposals expire/invalidate on Level or dependency changes.

## 16. Performance and abuse

- [ ] Checked arithmetic for all dimension/reserve/capacity calculations.
- [ ] Bounded upload, decoded pixel, proposal, simulation, and batch budgets.
- [ ] Brush commits transfer compact runs/chunks rather than entire Level when practical.
- [ ] Artwork blobs are content-addressed and structurally shared across revisions.
- [ ] Expensive InsideShape/Divide/migration jobs are cancellable and rate-limited.
- [ ] Client cannot request arbitrary server filesystem paths.
- [ ] Authorization separates authoring, library maintenance, batch migration, and publication.
