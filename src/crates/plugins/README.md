# cardinal-plugins

The middleware runtime.

## What it does

- Tracks builtin middleware (`RestrictedRouteMiddleware`, etc.) and user-supplied WASM plugins inside `PluginContainer`.
- Executes middleware chains via `PluginRunner`, respecting global order and destination-scoped middleware.
- Wraps WASM modules by delegating to `cardinal-wasm-plugins`.

## Adding middleware

### Rust

```rust
struct MyInbound;

#[async_trait::async_trait]
impl RequestMiddleware for MyInbound {
    async fn on_request(&self, session: &mut Session, backend: Arc<DestinationWrapper>, ctx: Arc<CardinalContext>) -> Result<MiddlewareResult, CardinalError> {
        // inspect/modify session, backend, ctx
        Ok(MiddlewareResult::Continue)
    }
}
```

Register it by inserting into `PluginContainer` during bootstrap (either by editing the defaults or supplying a provider factory).

### WASM

1. Compile an AssemblyScript or Rust-compiled WASM file.
2. Add it to configuration:

```toml
[[plugins]]
wasm = { name = "my-filter", path = "filters/my_filter.wasm" }
```

The runner validates exports (`handle`, `__new`) and executes it in inbound or outbound mode depending on where it’s registered.
