# Level Editor UI Shell and Integration Plan

## Delivered shell slice

The browser editor now has a desktop-first three-region shell:

- **Primary sidebar (left):** Shape Templates, Blame, Project Explorer, Comments, and Collaboration Session. Active tabs expand to show icon and label; inactive tabs remain icon-only.
- **Secondary sidebar (right):** Inspector, Level Structure, and Level Configuration. Canvas, marquee, and Explorer selections focus Inspector without reopening a deliberately hidden panel. Inspector section collapse state is user-scoped browser storage.
- **Center canvas:** existing real-time editable Canvas 2D renderer, with floating primary and contextual tool groups above it. The context group is omitted when the active primary workflow has no registered context actions. The status line shows the active theme source and can reset a user override to the project default.

The shell deliberately preserves the current authoritative command/RPC path. It does not create a second local level model.

Shape Studio is a focused modal with saved shapes, the editable grid, and properties/actions as separate regions. Image Studio imports bounded static PNG, JPEG, and WebP files into a project-scoped server catalog; a copied image can also be pasted while the studio is open. The server detects bytes rather than trusting client MIME data, verifies dimensions, rejects animated PNG/WebP, computes a BLAKE3 digest, and gates catalog/content reads behind project view access and imports behind `edit_timeline`. In the current MVP service those bytes remain process-memory only; they are shared among connected users but are not durable across a server restart.

## Current limitations, kept explicit

- The rendering backend is still **Canvas 2D**, not WebGL. It continues to redraw real-time editing and collaboration state through the existing canvas pipeline.
- Comments have no domain model, RPC messages, or persistence endpoint yet. The Comments tab is reserved rather than pretending activity history is a comment thread.
- Homogeneous multi-selection now exposes the common inspector state and mixed fields (`-`); the existing transform actions remain group-aware. General broadcast property mutation needs explicit compound command support before it can be offered.
- Strip Map and Clear All are visible but disabled: both need atomic, validated timeline commands rather than a client-side loop of destructive requests.
- The repository contains no Unity project/source or native bridge. The web app is a wasm/Yew client only.

## Safe delivery phases

### 1. Complete the browser shell

Keep the current Canvas 2D renderer while refining the tab-registration API, replacing remaining legacy panel copy, and adding a persisted comment model only after server support exists. Add atomic command kinds for map trim/clear before enabling those controls.

### 2. Introduce a renderer boundary, then WebGL

Extract a renderer-neutral scene projection from `EditorModel`/the snapshot and keep Canvas 2D as the reference fallback. Implement WebGL behind a runtime feature flag, feed it the same projection and interaction hit-test data, and compare it against Canvas 2D on fixed snapshots before switching the default. The timeline/RPC layer remains unchanged and authoritative throughout the migration.

### 3. Complete durable image artifacts and Image Studio operations

The MVP now has a narrow project-scoped static PNG/JPEG/WebP catalog with upload/download permissions, server-derived dimensions, BLAKE3 content hashes, and immutable template metadata. Replace its in-memory bytes with a durable metadata/blob store before treating it as persistent across restarts. Then add server-produced pixelation derivatives and explicit level-asset commands; do not persist browser-only image transformations or attach image bytes to the timeline.

### 4. Define the Unity/native integration contract

Before implementing a native client, add versioned server artifacts and source bindings around `oreak-legacy`:

1. upload/import a legacy LevelData JSON artifact;
2. build/export a validated legacy artifact from an authoritative revision;
3. publish/download immutable artifact versions;
4. request runtime verification with a correlation ID and result log.

A separately versioned Unity-side package can then consume the artifact, install a temporary level-0 override for Real-Test, launch verification, and restore the previous runtime state even on failure. That package must be implemented in the Unity repository (or supplied as a bridge package); there are no C# sources in this checkout to call today.

## Ownership invariant

The server validates and orders level mutations. Browser Canvas 2D/WebGL renderers and any future Unity/native client are presentation or verification clients only. They may preview a draft, but may not become an independent source of truth.
