# cardinal-base

`cardinal-base` underpins the gateway’s dependency graph.  It hosts the DI container, routing utilities, and destination resolver used everywhere else.

## Components

- **`CardinalContext`** – owns the active `CardinalConfig`, tracks provider registrations, and constructs providers on demand.  Scope-aware (`Singleton` vs `Transient`) with cycle detection to keep async factories honest.
- **Provider traits** – implement `Provider` for any type you want to resolve later.  Register with `register`, `register_with_factory`, or `register_singleton_instance`.
- **`CardinalRouter`** – small wrapper around `matchit::Router`; used by destinations to match HTTP method + path and extract path parameters.
- **`DestinationContainer`** – builds `DestinationWrapper`s from config, supplies per-destination middleware lists, and picks a backend by path segment or subdomain.

## How it fits

At runtime the flow looks like this:

```
CardinalBuilder
   └─ registers providers in CardinalContext
        ├─ DestinationContainer (maps host/path → backend + middleware)
        ├─ PluginContainer (middleware registry)
        └─ any user-provided services
```

When `cardinal-proxy` handles a request it grabs the `DestinationContainer` and (if routes were declared) queries the `CardinalRouter` to validate the path and extract parameters.  Middleware lists are pulled from each `DestinationWrapper` and fed to the plugin runner.

## Adding new providers

```rust
struct Metrics;

#[async_trait::async_trait]
impl Provider for Metrics {
    async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
        // optional: use ctx.config to decide setup
        Ok(Metrics)
    }
}

context.register::<Metrics>(ProviderScope::Singleton);
```

Later, middleware or infrastructure code can ask for `context.get::<Metrics>().await?` and work with the shared instance.
