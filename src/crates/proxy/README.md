# cardinal-proxy

Pingora integration for Cardinal.  `CardinalProxy` implements `ProxyHttp` and delegates request/response processing to the rest of the stack.

## Key pieces

- `CardinalContextProvider`: resolves an `Arc<CardinalContext>` from a `Session`.  The default `StaticContextProvider` always returns the same context; more advanced deployments can plug in host-aware providers.
- `RequestContext`: per-request cache of the resolved context, destination backend, and `PluginRunner`.
- Middleware execution: `PluginRunner::run_request_filters` / `run_response_filters` are invoked at the right phases, so both Rust and WASM middleware can observe or mutate traffic.

## Lifecycle

1. Provider resolves a context.
2. `DestinationContainer` selects the backend based on path/host.
3. Request middleware runs; it can short-circuit with `MiddlewareResult::Responded`.
4. Pingora connects to the upstream origin.
5. Response middleware runs; optional logging is performed.

Consumers rarely touch this crate directly—`Cardinal` handles wiring—but understanding it is useful when implementing custom providers or debugging proxy behaviour.
