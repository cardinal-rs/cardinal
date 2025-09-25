# cardinal-config

Configuration loader and validator for the gateway.  It uses the `config` crate to merge sources, then maps them into strongly typed builders.

## Features

- Load from multiple TOML files and `CARDINAL__...` environment variables
- Validate server address formats, whitelisted methods, and plugin references
- Emit descriptive `ConfigError`s when validation fails

## API sketch

```rust
let paths = vec!["config/base.toml".into(), "config/local.toml".into()];
let cfg = cardinal_config::load_config(&paths)?;
assert_eq!(cfg.server.address, "127.0.0.1:1704");
```

`CardinalConfigBuilder` and friends remain available if you need to construct configs programmatically (e.g., tests or higher-level control planes).
