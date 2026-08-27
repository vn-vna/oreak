# Oreak Plugin Runner

`oreak-plugin-runner` is a native process boundary for untrusted Lua 5.4
plugins. It reads one JSON request per line from stdin and writes one JSON
response per line to stdout. A plugin is loaded from source text in a `load`
request; the runner never accepts a source path.

The plugin chunk must return a table whose keys exactly match the exports in
its manifest. Hook input is a recursively read-only JSON-like value. Hook
output is deserialized as `oreak_plugin_api::HookOutput`, validated, and
returned as declarative proposals, diagnostics, render primitives, or generated
artifacts. The runner never applies proposals to a core document.

The Lua environment only contains selected base functions plus cloned `table`,
`string`, `math`, and `utf8` libraries. It does not expose `_G`, `io`, `os`,
`package`, `debug`, `require`, `dofile`, `loadfile`, `load`, `collectgarbage`,
`print`, raw table access, metatable mutation, coroutines, or math randomness.
No host callbacks provide network, filesystem, database, browser, or GPU
access.

## Security boundary and exact limits

The OS process is the hard security and termination boundary. The parent must
start one runner with ordinary least-privilege OS controls, enforce its own
absolute deadline, and kill the process if it stops responding. `shutdown` is
only a cooperative clean exit.

Within the process, mlua's custom allocator rejects Lua VM allocations above
the configured memory limit. That limit does not include Rust allocations for
the NDJSON request, source string, serde conversion, response serialization,
the executable, or other process overhead.

Lua instruction hooks account for interpreted VM instructions at periodic
checkpoints. They do not count work performed inside Lua's C standard-library
functions. A hook error can be caught by Lua `pcall`, so the runner also records
budget exhaustion out of band and rejects the invocation even if Lua catches
the error.

The wall-time deadline is checked by the instruction hook and again after Lua
returns. mlua does not provide a hard asynchronous interrupt for Lua 5.4: a
long parser/compiler operation or C library call can run past the deadline
before control returns. This is why the parent process deadline and kill are
required for a hard guarantee.

The output-byte limit is checked after conversion to the typed declarative
output. Lua-side construction is constrained by the VM memory limit, but only
the parent process limit covers all transient host allocations.
