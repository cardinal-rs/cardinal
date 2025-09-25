# cardinal-wasm-plugins

`cardinal-wasm-plugins` is the host runtime that executes WebAssembly middleware inside Cardinal.  It is responsible for loading modules, wiring the import surface, and running the guest code in either inbound or outbound mode.

## Execution model

```
CardinalProxy
  │
  ├─ inbound middleware → WasmRunner (ExecutionType::Inbound)
  │      • read-only access to headers/query/body
  │      • returns `should_continue` (0/1)
  │
  └─ outbound middleware → WasmRunner (ExecutionType::Outbound)
         • can mutate response headers/status
         • observes the request context as well
```

The canonical entry point exported by a WASM module is `handle(ptr: i32, len: i32) -> i32`.  The return value maps to `should_continue` (1) or `responded` (0).  `__new` (AssemblyScript) or a compatible allocator must also be present so the host can write the request body into guest memory when needed.

## Core types

- `WasmPlugin`: loads bytes from disk (or memory), validates required exports, and remembers the configured memory/handle symbols.
- `WasmInstance`: wraps the instantiated module, guest memory, and `FunctionEnv<ExecutionContext>` so host imports can mutate state.
- `ExecutionContext`: enum with `Inbound` and `Outbound` variants.  Inbound mode surfaces `ExecutionRequest` (headers, query string, optional body).  Outbound mode extends that with `ExecutionResponse` (mutable `resp_headers`, `status`).
- `WasmRunner`: orchestrates a run—copying the current context into the guest, invoking `handle`, and harvesting results.

## Host imports

| Import | Mode | Description |
|--------|------|-------------|
| `get_header(name, out_ptr, out_cap)` | inbound + outbound | copy a header value into guest memory; returns byte count or `-1` |
| `get_query_param(key, out_ptr, out_cap)` | inbound + outbound | similar to `get_header`, but for query parameters |
| `set_header(name, value)` | outbound only | stage a response header to be written back to Pingora |
| `set_status(code)` | outbound only | override the HTTP status sent to the client |
| `abort(code, msg_ptr, msg_len)` | both | abort execution; surfaces as `CardinalError::InternalError(InvalidWasmModule)` |

Inbound code is intentionally read-only: it can veto a request by returning 0, but it cannot mutate headers/state on the way to the upstream backend.

## Fixture-driven tests

The crate’s unit tests load fixtures from `tests/wasm-plugins/<case>`:

```
📁 tests/wasm-plugins/
  ├─ allow/
  │   ├─ plugin.ts (AssemblyScript source)
  │   ├─ plugin.wasm (compiled)
  │   ├─ incoming_request.json
  │   └─ expected_response.json
  ├─ inbound-allow/
  ├─ inbound-block/
  └─ outbound-tag/
```

`incoming_request.json` feeds `ExecutionContext`, while `expected_response.json` specifies:

```json
{
  "execution_type": "outbound",  // or "inbound"
  "should_continue": true,
  "status": 200,
  "resp_headers": {
    "x-example": "value"
  }
}
```

For inbound tests, `status`/`resp_headers` must be omitted; the runner enforces this to keep fixtures honest.

## authoring WASM middleware

1. Write AssemblyScript (or any language that compiles to WASM) using the imports above.  AssemblyScript examples live alongside the fixtures.
2. Compile with `tests/wasm-plugins/compile.sh` or equivalent tooling (`npx asc plugin.ts -o plugin.wasm --optimize --exportRuntime`).
3. Reference the `.wasm` file in `cardinal-config`:

```toml
[[plugins]]
wasm = { name = "audit", path = "filters/audit/plugin.wasm" }
```

During runtime, `PluginContainer` loads the module, and `PluginRunner` invokes it in the appropriate phase.

## Error handling

`WasmRunner::run` returns `CardinalError` variants when:

- required exports are missing (`InvalidWasmModule`)
- the guest traps or calls `abort`
- memory writes fail

Callers should treat these errors as fatal—Cardinal responds with `500` and logs the failure.

## Extending the runtime

- New host imports can be added under `src/host/` (mirroring the existing modules).  Update `make_imports` so the appropriate functions are only exposed in the modes that make sense.
- To support alternative languages/runtimes, ensure they can export the same C ABI (`handle`, allocator).  The current implementation assumes linear memory via Wasmer 6.
