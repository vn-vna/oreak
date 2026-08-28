# Oreak

Oreak is a collaborative, browser-first level editor for Sand Block levels. It
is a Rust workspace with an Axum/jsonrpsee server, a Yew client, and a
deterministic core shared by native and WebAssembly targets.

The product uses one append-only collaborative timeline per level rather than
Git-like level branches. Immutable review snapshots and release artifacts are
created from that timeline. History is never rewritten: Undo and restoration
are represented by new events, which keeps audit, contribution, and blame data
explainable.

## Implemented

- `oreak-core`: grids, Blocks, Blinds, connected shapes, decorators, atomic
  commands, deterministic hashes, blame, conflict-safe Undo, and an isolated
  Sandbox session.
- `oreak-protocol` and `oreak-server`: typed JSON-RPC snapshots, bounded
  paginated history, commands, Undo, and subscriptions over HTTP/WebSocket with
  authenticated, project-scoped, server-ordered level timelines.
- `oreak-web`: a desktop-first Yew PWA with dynamically sized maps, zoom/pan,
  top mode tabs, bottom action tools, split sidebar tabs, ordered multi-selection
  and grouped movement, a shared Block/Blind Shape Designer, solid connected
  entity footprints, project shape samples, Blind isolation, live collaboration,
  login/registration, offline snapshot fallback, command palette,
  notifications, and cell/entity blame display.
- `oreak-project`: workspace/project permissions, review records, immutable
  artifacts, release channels, audit records, and effective-dated KPI policies.
- `oreak-legacy`: strict, loss-preserving Unity LevelData parsing and export,
  including DataCodec Base64/XOR/CRC32/GZip handling.
- `oreak-plugin-api` and `oreak-plugin-runner`: declarative plugin contracts,
  exact-version codec release gates, and a budgeted Lua 5.4 process boundary.
- MVP REST APIs for email/password sessions, workspaces, projects, levels,
  project shape catalogs, memberships, audit summaries, and release channels.

The Unity project remains a read-only behavior reference. See
`docs/ROADMAP.md` for product slices and `docs/EDITOR_PARITY.md` for the
element-by-element Unity port status.

## Run locally

Requirements are Rust 1.85 or newer, the `wasm32-unknown-unknown` target, and
Trunk exactly 0.21.14. Initialize the local environment and create the ignored
`bootstrap.local.toml` settings file:

```text
cargo run -p bootstrap -- init
```

For unattended setup, explicitly accept prerequisite installation and default
settings:

```text
cargo run -p bootstrap -- init --non-interactive --yes
```

Run both development processes with health-gated startup and shared lifecycle
management:

```text
cargo run -p bootstrap -- dev
```

Open `http://127.0.0.1:8080`. The generated Trunk configuration under
`target/bootstrap` proxies `/api/` and the `/rpc` WebSocket to the backend while
preserving Oreak's same-origin RPC policy. Ctrl+C or an unexpected child exit
stops both process groups.

Other common commands:

```text
cargo run -p bootstrap -- server --listen 127.0.0.1:3000
cargo run -p bootstrap -- frontend --backend 127.0.0.1:3000 --open
cargo run -p bootstrap -- build --release
cargo run -p bootstrap -- verify
```

`scripts/bootstrap.ps1` remains as a compatibility shim for its former
`-Listen`, `-Release`, `-Verify`, and `-NoStart` arguments.

Server configuration:

- `OREAK_LISTEN`: listen address; defaults to `127.0.0.1:3000`.
- `OREAK_WEB_DIST`: compiled PWA directory; defaults to `apps/oreak-web/dist`.
- `OREAK_SECURE_COOKIES`: set to `true` behind HTTPS to add the `Secure`
  session-cookie attribute.
- `RUST_LOG`: server tracing filter; defaults to `info`.
- `OREAK_RELEASE`: server release-mode boolean.
- `OREAK_FRONTEND_LISTEN`, `OREAK_BACKEND`, `OREAK_FRONTEND_OPEN`, and
  `OREAK_FRONTEND_RELEASE`: frontend overrides.

Bootstrap settings use CLI options first, then environment variables, then
`bootstrap.local.toml`, then built-in defaults. Non-loopback listen addresses
must also pass `--allow-public-bind`.

## Development

`cargo run -p bootstrap -- verify` mirrors the CI formatting, workspace check,
Clippy, tests, full WebAssembly check, and Trunk release build. Cargo's `--`
separator is required so arguments after it are sent to `bootstrap` rather than
to Cargo.

## MVP limitations

- Users, sessions, workspaces, projects, and level timelines are in process
  memory and are lost when the server restarts.
- RPC resolves project/level ownership from the authenticated HttpOnly session,
  replaces client actor/timestamp metadata, and enforces view/edit capabilities.
  Production CSRF/proxy-origin policy still requires deployment configuration.
- The browser edits map cells and an expanded Block/Blind parity slice including
  resize, navigation, shape samples, multi-selection, grouped movement,
  decorators, and size-1 Blind Brush behavior. Merge, grouped transforms beyond
  movement, larger Brush tips, legacy workflows, governance, and plugins are not
  all wired into browser workflows.
- History hydration is capped at 4,096 events per browser connection for the
  in-memory MVP; levels beyond that cap fail explicitly instead of loading an
  unbounded activity log.
- Email verification, password recovery, durable drafts, PostgreSQL,
  IndexedDB command recovery, collaborator presence, and production deployment
  hardening remain outstanding.
