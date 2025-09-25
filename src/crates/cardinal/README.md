# cardinal (library)

This crate assembles the gateway: feeding configuration into `CardinalContext`, registering default providers, and booting the Pingora service.

## Responsibilities

- `CardinalBuilder` wires a `CardinalContext`, handles provider registration, and exposes extension hooks.
- `Cardinal` wraps the built context and knows how to run the proxy.
- Optional `with_context_provider` lets callers inject a custom `CardinalContextProvider` (e.g. host-based lookup) while the default remains a static provider.

## Quick start

```rust
let config = cardinal_config::load_config(&["config/local.toml".into()])?;

let gateway = Cardinal::builder(config)
    .register_provider::<Telemetry>(ProviderScope::Singleton)
    .with_context_provider(my_provider) // optional
    .build();

gateway.run()?;
```

`CardinalBuilder::new_empty` is available if you want to bypass default registrations and compose the provider graph manually (useful for tests).

## When to touch this crate

- You’re adding another default provider that should always be present.
- You need to expose a new builder hook so downstream users can plug in custom logic before the proxy starts.
- You’re evolving the integration with Pingora (e.g., swapping context providers, tweaking runtime options).
