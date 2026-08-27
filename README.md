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
- `oreak-web`: a desktop-first Yew PWA with an `8 x 8` canvas editor, live
  collaboration, login/registration, workspace/project/level selection, offline
  snapshot fallback, command palette, and blame display.
- `oreak-project`: workspace/project permissions, review records, immutable
  artifacts, release channels, audit records, and effective-dated KPI policies.
- `oreak-legacy`: strict, loss-preserving Unity LevelData parsing and export,
  including DataCodec Base64/XOR/CRC32/GZip handling.
- `oreak-plugin-api` and `oreak-plugin-runner`: declarative plugin contracts,
  exact-version codec release gates, and a budgeted Lua 5.4 process boundary.
- MVP REST APIs for email/password sessions, workspaces, projects, levels,
  memberships, audit summaries, and release channels.

The Unity project remains a read-only behavior reference. See
`docs/ROADMAP.md` for integration and parity gates that are not complete.

## Run locally

Requirements are Rust 1.85 or newer, the WebAssembly target, and Trunk 0.21.

On Windows, the development bootstrap installs missing local prerequisites,
builds the server and PWA, configures the same-origin asset path, and starts
the server:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap.ps1
```

Useful options:

- `-Listen 127.0.0.1:3100` changes the development address.
- `-Release` builds and runs optimized binaries.
- `-Verify` runs formatting, checks, tests, and Clippy before building.
- `-NoStart` builds the stack without launching the server.

```text
rustup target add wasm32-unknown-unknown
cargo install --locked trunk@0.21.14
cd apps/oreak-web
trunk build --release
cd ../..
cargo run -p oreak-server
```

Open `http://127.0.0.1:3000`. The server serves `apps/oreak-web/dist` by
default, including the PWA fallback, and exposes JSON-RPC at `/rpc`.

Server configuration:

- `OREAK_LISTEN`: listen address; defaults to `127.0.0.1:3000`.
- `OREAK_WEB_DIST`: compiled PWA directory; defaults to `apps/oreak-web/dist`.
- `OREAK_SECURE_COOKIES`: set to `true` behind HTTPS to add the `Secure`
  session-cookie attribute.

## Development

```text
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p oreak-core -p oreak-project -p oreak-plugin-api -p oreak-protocol -p oreak-web --target wasm32-unknown-unknown
cd apps/oreak-web
trunk build --release
```

## MVP limitations

- Users, sessions, workspaces, projects, and level timelines are in process
  memory and are lost when the server restarts.
- RPC resolves project/level ownership from the authenticated HttpOnly session,
  replaces client actor/timestamp metadata, and enforces view/edit capabilities.
  Production CSRF/proxy-origin policy still requires deployment configuration.
- The browser edits selected project levels, but its current surface is the
  `8 x 8` map editor. Placeable, decorator, legacy, governance, and plugin
  domains are not all wired into browser workflows.
- History hydration is capped at 4,096 events per browser connection for the
  in-memory MVP; levels beyond that cap fail explicitly instead of loading an
  unbounded activity log.
- Email verification, password recovery, durable drafts, PostgreSQL,
  IndexedDB command recovery, collaborator presence, and production deployment
  hardening remain outstanding.
