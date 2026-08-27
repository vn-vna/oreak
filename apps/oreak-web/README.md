# oreak-web

Desktop-first Yew 0.23 PWA shell for the Oreak technical editor. The map editor
uses `oreak-core` locally and the generated `OreakRpcClient` from
`oreak-protocol` for authoritative snapshots, commands, undo, and level-event
subscriptions. Draft snapshots and theme overrides are stored in browser local
storage when collaboration is unavailable.

```text
rustup target add wasm32-unknown-unknown
cd apps/oreak-web
trunk build --release
cd ../..
cargo run -p oreak-server
```

The app bootstraps the HttpOnly session, presents login/registration when
needed, and lets the user select or create a project and level before opening
the editor. The RPC endpoint is `ws(s)://<current-host>/rpc`; serving the
compiled PWA from `oreak-server` keeps the editor and WebSocket on the same
origin. Separate `trunk serve` development requires a WebSocket-capable reverse
proxy to the server.

The client subscribes first, anchors bounded paginated history to the initial
subscription snapshot, and buffers later events until hydration completes.
Activity and cell blame therefore include prior server events and exact Undo
provenance. Hydration currently has a 4,096-event safety cap, and presence
counts are not exposed. Draft snapshots are scoped by authenticated user,
project, and level. Project capabilities make the editor read-only unless the
exact `edit_timeline` capability is present.
