# Level Editor UI Shell and Integration Plan

## Delivered shell slice

The browser editor now has a desktop-first three-region shell:

- **Primary sidebar (left):** Shape Templates, Blame, Project Explorer, Comments, and Collaboration Session. Active tabs expand to show icon and label; inactive tabs remain icon-only.
- **Secondary sidebar (right):** Inspector, Level Structure, and Level Configuration. Canvas, marquee, and Explorer selections focus Inspector without reopening a deliberately hidden panel. Inspector section collapse state is user-scoped browser storage.
- **Center canvas:** existing real-time editable Canvas 2D renderer, with floating primary and contextual tool groups above it. The context group is omitted when the active primary workflow has no registered context actions. The status line shows the active theme source and can reset a user override to the project default.

The shell deliberately preserves the current authoritative command/RPC path. It does not create a second local level model.

Shape Studio is a focused modal with saved shapes, the editable grid, and properties/actions as separate regions. Image Studio imports bounded static PNG, JPEG, and WebP files into a project-scoped server catalog; a copied image can also be pasted while the studio is open. The server detects bytes rather than trusting client MIME data, verifies dimensions, rejects animated PNG/WebP, computes a BLAKE3 digest, and gates catalog/content reads behind project view access and imports behind `edit_timeline`. In the current MVP service those bytes remain process-memory only; they are shared among connected users but are not durable across a server restart.

### Image-to-sand workflow

1. Import or paste a static image (512 KiB maximum, up to 1024 pixels per axis). The original is retained unchanged. The palette converter compares server-decoded RGBA and palette-only output at the same dimensions; it offers enabled colors, alpha threshold, background, and optional Floyd–Steinberg dithering. Advanced palette mapping exposes up to 16 dominant source-color clusters (5-bit RGB buckets): drag chips into enabled game-color groups or use their keyboard-accessible dropdowns. Unassigned buckets keep automatic matching; explicit mappings use original RGB even when a background is selected, and never recolor transparent pixels. Saving creates a new immutable prepared variant, recomputed by the server. Prepared cards show authenticated, max-256-pixel PNG thumbnails generated from the converted indices, not the original image.
2. Choose a prepared variant and enter Apply Image. A temporary **Apply Image** left-sidebar tab and **Select Pool / Adjust Image / Cancel / Apply** secondary tools replace normal editing until exit. Select Pools by click, Shift-click, or marquee; drag/resize the image or use numeric placement and Fit/Cover controls.
3. Choose nearest or area sampling, source-pixel block size, transparency preserve/erase, and existing or 1–32 pixels-per-cell resolution. Placement spans the selected Pools in shared grid coordinates and clips to their occupied cells. Palette changes return to the converter and save another variant. Resolution changes resample the selected Pools, including their existing pixels and guides.
4. Apply submits one server-authoritative command with the prepared asset ID and captured Pool states. The server rejects stale targets atomically; a successful operation creates one undo step. Cancel before submission writes no level data; closing a submitted operation cannot retract its in-flight command.

Prepared images share the MVP's **in-memory-only** lifetime. Bounds are 64 prepared images / 16 MiB indexed pixels per project and 64 selected Pools / 1,000,000 source and destination Pool pixels per application. Image edits are also checked against an 8 MiB serialized response/history budget before commit; reduce the selected coverage or resolution if an edit exceeds it. History pages honor both event count and byte limits. The browser previews with the shared converter/projector; it does not upload authoritative transformed pixels. Raw encoded source dimensions are preserved; EXIF orientation and embedded color profiles are not applied by the converter.

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

The MVP now includes original and prepared palette-image catalogs, full-resolution conversion, and an atomic image-to-Pool command. Replace its in-memory catalogs with a durable metadata/blob store before treating assets as persistent across restarts. Encoded original image bytes stay outside the timeline; current replayable image commands contain server-resolved indexed pixels and settings. A future content-addressed artifact store can replace repeated indexed payloads with durable references without trusting browser-only transformations.

### 4. Define the Unity/native integration contract

Before implementing a native client, add versioned server artifacts and source bindings around `oreak-legacy`:

1. upload/import a legacy LevelData JSON artifact;
2. build/export a validated legacy artifact from an authoritative revision;
3. publish/download immutable artifact versions;
4. request runtime verification with a correlation ID and result log.

A separately versioned Unity-side package can then consume the artifact, install a temporary level-0 override for Real-Test, launch verification, and restore the previous runtime state even on failure. That package must be implemented in the Unity repository (or supplied as a bridge package); there are no C# sources in this checkout to call today.

## Ownership invariant

The server validates and orders level mutations. Browser Canvas 2D/WebGL renderers and any future Unity/native client are presentation or verification clients only. They may preview a draft, but may not become an independent source of truth.
