# Architecture

## Product boundaries

Oreak is organized around projects. A project owns levels, permissions, audit
policy, KPI policy, release channels, palettes, assets, plugin versions, codec
certifications, and its default theme. Personal and organization workspaces can
contain projects, but edit authority is always granted at the project boundary.

Each level has one mutable collaborative draft and one append-only command
timeline. Review candidates, approved revisions, LevelData artifacts, and
release-channel pointers are immutable records derived from that timeline.

## Shared core

`oreak-core` must compile for native Rust and `wasm32-unknown-unknown`. It must
not depend on a database, filesystem, browser API, wall clock, implicit random
source, or async runtime. IDs and timestamps are supplied by the caller.

The browser uses the core for immediate validation and previews. The server
uses the same core to order and revalidate committed commands. A server result
is authoritative, while matching deterministic hashes detect implementation or
plugin divergence.

## History and blame

History is application data, not source control. Every accepted command stores
its actor, timestamp, sequence, inverse, touched targets, and before/after
values. No-op commands create no event. Undo appends a compensating command and
never deletes an earlier event.

Blame is a derived provenance query. The core tracks logical cells, entity
ownership and fields, decorators, and Blind edits. Project settings, plugin
data, and release-channel changes remain in their separate audit and review
models rather than the level command timeline.

Security audit records and KPI contribution events remain separate from the
command timeline. Both may reference command sequence ranges without copying
the complete editing payload.

## Legacy compatibility

The canonical Oreak schema and Unity LevelData are separate representations.
The legacy adapter retains source templates and patches only editor-owned fields
so entity order, opaque payload properties, compact rows, and unchanged
DataCodec values survive round trips. Runtime custom plugin entities block a
release unless their pinned plugin version has a certified LevelData codec.

## Rendering and plugins

Yew renders application chrome as HTML and the current map surface on a canvas.
Placeables, simplified Blind artwork, decorators, and connection-scoped remote
cursors render on the browser canvas; richer artwork and guide rendering remain
parity gates. Themes use semantic tokens;
untrusted plugins cannot inject arbitrary CSS, DOM, network, filesystem,
database, or GPU calls.

Lua 5.4 plugins run behind a capability API and propose normal validated
commands. The implemented native runner executes each plugin behind an OS
process boundary with memory, instruction, time, and output budgets. Browser
worker orchestration and server integration are not implemented yet.

## Current deployment boundary

`oreak-server` combines REST application routes, JSON-RPC collaboration, and
static PWA hosting. Its current stores are deliberately process-local. REST
sessions use Argon2id password hashes and opaque HttpOnly cookies. JSON-RPC
requests carry explicit project/level scope; the transport derives identity
from the session cookie, validates project capabilities on every call, and
replaces client actor and timestamp metadata. Browser-origin RPC upgrades must
match the request host. PostgreSQL persistence, a deployment-aware CSRF/proxy
origin policy, email verification/recovery, and production operational controls
are required before deployment outside an isolated MVP environment.
