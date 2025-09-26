# Cardinal Gateway

Cardinal is a programmable reverse proxy layered on top of Cloudflare’s [Pingora](https://github.com/cloudflare/pingora).  The workspace is organised as a set of Rust crates that separate core concerns: configuration, dependency injection, middleware execution (Rust and WASM), the Pingora integration, and the CLI surface.

## Architecture snapshot

```
┌──────────────┐       ┌─────────────┐       ┌──────────────┐       ┌──────────────┐
│ cardinal-cli │──cfg──▶ cardinal    │──ctx──▶ cardinal-proxy │──RPC──▶ upstream backends │
└──────────────┘       │  (builder) │       └──────────────┘       └──────────────┘
                       │            │                │
                       │            │                ├─ uses cardinal-plugins
                       │            │                │    ├─ Rust middleware
                       │            │                │    └─ WASM middleware via cardinal-wasm-plugins
                       │            │                └─ resolves dependencies from cardinal-base
                       └────────────┘
```

* `cardinal-base` provides `CardinalContext`, a DI container that knows how to construct and cache providers.
* `cardinal-config` loads/merges TOML + ENV into `CardinalConfig`, enforcing validation rules.
* `cardinal` wires the context, registers default providers (destinations, plugins), and exposes `CardinalBuilder`/`Cardinal`.
* `cardinal-proxy` implements Pingora’s `ProxyHttp` trait.  It resolves a context through a `CardinalContextProvider`, selects a backend with `DestinationContainer`, and drives middleware using `PluginRunner`.
* `cardinal-plugins` stores builtin middleware and wraps WASM modules through `cardinal-wasm-plugins`.

## Boot sequence

```rust
let config_paths = vec!["config/default.toml".into(), "config/local.toml".into()];
let config = cardinal_config::load_config(&config_paths)?;

let cardinal = Cardinal::builder(config)
    .with_context_provider(Arc::new(StaticContextProvider::new(context.clone())))
    .register_provider::<MyProvider>(ProviderScope::Singleton)
    .build();

cardinal.run()?;
```

1. Configuration is parsed and validated.
2. `CardinalBuilder` initialises a `CardinalContext`, registers providers (DestinationContainer, PluginContainer, plus anything you add), and optionally accepts a custom `CardinalContextProvider`.
3. `Cardinal::run()` spins up Pingora, builds a `CardinalProxy`, and binds to the configured address.
4. For each request, the proxy resolves a context, rewrites the downstream path (if `force_path_parameter = true`), executes request middleware, connects upstream, then executes response middleware.

## Configuration anatomy

```toml
[server]
address = "127.0.0.1:1704"
force_path_parameter = true
log_upstream_response = false
global_request_middleware = []
global_response_middleware = []

[destinations.posts]
name = "posts"
url = "127.0.0.1:9001"
routes = []              # optional; used by RestrictedRouteMiddleware
middleware = []          # destination-scoped middleware references

[[plugins]]
builtin = { name = "RestrictedRouteMiddleware" }
# wasm = { name = "foo", path = "filters/foo.wasm" }
```

* `server` controls listener behaviour and global middleware order.
* `destinations` map a logical service (e.g., `posts`) to an upstream origin.  Routes are optional; when present they feed the route-restriction middleware.
* `plugins` register Rust or WASM middleware by name.

Environment variables follow the `CARDINAL__SECTION__key=value` convention.  All config is merged in order, so later files override earlier ones.

## Request lifecycle

1. **Resolve context** – `CardinalContextProvider::resolve(session)` returns the `Arc<CardinalContext>` to use for this request.  The default provider always returns the same context; more advanced deployments can override this (e.g., SNI/Host-based lookups).
2. **Destination routing** – `DestinationContainer::get_backend_for_request` inspects the request path or host (depending on `force_path_parameter`) to choose a backend.
3. **Request middleware** – `PluginRunner::run_request_filters` runs global middlewares followed by destination-scoped ones.  Middleware can short-circuit by returning `MiddlewareResult::Responded`.
4. **Upstream call** – The proxy opens a connection via Pingora, adjusts host/SNI headers, and forwards the request.
5. **Response middleware** – Global + destination-scoped response middleware run before the response returns to the client.

## Extending the gateway

- **Custom providers:** implement `Provider` and register through `CardinalBuilder::register_provider` or `register_provider_with_factory` to make new services available to middleware.
- **Alternate context selection:** implement `CardinalContextProvider` (e.g., a `DashMap<HostKey, Arc<CardinalContext>>`) and pass it to `CardinalBuilder::with_context_provider`.
- **New middleware:**
  * Rust: implement `RequestMiddleware` / `ResponseMiddleware`, register in `PluginContainer` (either by editing default registration or supplying your own container via provider).
  * WASM: place AssemblyScript or Rust-generated WASM modules under `tests/wasm-plugins`, reference them in configuration with a `path`, and they’ll be loaded at runtime.

## Running & testing

```bash
# Format and lint
auto_fmt() { cargo fmt; cargo clippy --workspace --all-targets; }
auto_fmt

# Execute unit + integration tests
cargo test --workspace

# Launch the gateway
cargo run -p cli -- --config config/example.toml
```

Integration tests under `src/crates/cardinal/src/tests` spin up tiny HTTP servers via `tiny_http` and exercise the full pipeline, including WASM middleware.  Because those tests bind local ports, they’re skipped in CI environments without permission to open sockets; run them locally if you tweak the proxy or context-provider wiring.

## Contributing

1. Fork the repository and create a topic branch
2. Make your changes, adding tests where it makes sense
3. Run `cargo fmt`, `cargo clippy`, and `cargo test --workspace`
4. Submit a PR with a succinct explanation of the change and any relevant test output

We welcome issues for bug reports, architectural questions, or middleware ideas.  Please include reproduction steps or sample configurations when filing a bug.
