# Editor parity

The Unity editor under
`Assets/__Project/Scripts/Editor/LevelEditor` is the read-only behavioral
reference. A feature is complete only when its deterministic Rust domain,
Unity LevelData compatibility, collaborative protocol, and browser workflow are
all covered.

| Capability | Core | Legacy | Web | Remaining work |
| --- | --- | --- | --- | --- |
| Floor/Wall cells | Complete | Complete | Complete | Aggregate drag samples into one atomic stroke. |
| Block/Blind rendering and selection | Complete | Complete | Complete | Connected cells of one entity render as one solid outlined footprint; add richer artwork parity. |
| Block/Blind shape designer and project samples | Complete | Complete | Complete | Templates drag onto the canvas as one atomic placement; add sample reordering and authored collect properties. |
| Move/rotate/flip/delete entities | Complete | Complete for compatible metadata | Partial | Direct hold-drag and atomic ordered group move are complete; grouped rotate/flip/delete and SBLN transformations remain. |
| Map resize | Complete | Complete | Complete | Four content-aware edge handles emit one atomic command; destructive clipping remains intentionally rejected. |
| Merge and Blind resize | Missing | Partial | Missing | Port merge rules and SBLN topology transforms. |
| Block collect properties | Partial | Partial | Read-only summary | Add collaborative color/layer/radius/capacity/lock commands. |
| Capacity analysis/normalization | Missing | Partial | Missing | Port Unity analyzer and deterministic normalization plan. |
| Ice/Direction/KeyLocker/Glass | Partial | Partial | Partial | Glass is now Blind-only with typed timeline/RPC/browser controls and active/`_led.dt` tombstone legacy round-trip; generalize tombstones, then add movable badges, batch selection, and dedicated decorator history. |
| Blind paint/erase/fill | Complete | Complete | Complete | Add Block color sampling and mixed-color held gestures; local Blind isolation is complete. |
| Blind pixel resolution | Complete | Compatible through final pixels | Complete | Per-Pool 1-32 resolution changes resample deterministically and support exact Undo. |
| One-pixel stroke interpolation | Complete | Final pixels compatible | Complete | None for the size-1 tip. |
| Square/Round/Diamond tips | Missing | Final pixels compatible | Missing | Port tip settings and deterministic stamp rasterization. |
| Guide lines | Partial | Read-only preservation | Missing | Port snapping/rasterization and encode changed SBLN topology. |
| Divide | Missing | Missing | Missing | Port planner, metrics, preview, and application. |
| Sandbox cardinal movement | Complete | Not applicable | Missing | Add isolated browser Sandbox state and controls. |
| Sandbox analog movement/rules | Missing | Not applicable | Missing | Port path validation, release, KeyLocker, and Ice behavior. |
| Sand simulation | Missing | Not applicable | Missing | Port deterministic gravity, collection, capacity, and melt ticks. |
| Unity file import/export workflow | Adapter complete | Complete | Missing | Add bounded server APIs, compatibility report, upload, and download. |
| Pan/zoom/frame | Not applicable | Not applicable | Complete | Rectangular responsive canvas, wheel zoom, right/middle-button pan, toolbar zoom, and frame share one viewport transform. |
| Notification history | Not applicable | Not applicable | Complete | Toasts auto-hide visually while the header dropdown retains the latest 200 notices. |
| Presence and remote cursors | Not applicable | Not applicable | Complete | Add an automated two-browser acceptance test; roster and cursors are connection-scoped and actor-grouped in the UI. |
| Multi-selection | Complete for grouped move | Not applicable | Complete | Ctrl/Shift additive selection and drag marquee are complete; grouped rotate/flip/delete remain. |
| Panel workspace | Not applicable | Not applicable | Complete | Tool and inspector panels can be resized, docked, floated, hidden, and restored from View. |
| Level name/duration | Project domain | Not yet exported | Complete | Configuration is project-audited REST state rather than timeline content. |

## Delivery order

1. Block collect-layer properties and capacity normalization.
2. Unity LevelData import/export workflow.
3. Grouped rotate/flip/delete, decorator batch/tombstone parity, merge, and Blind resize.
4. Square/Round/Diamond Brush tips, guide lines, and Divide.
5. Browser Sandbox, followed by deterministic sand simulation.
