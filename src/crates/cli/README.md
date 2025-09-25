# cardinal-cli

`cardinal-cli` packages the gateway as a runnable binary.

## Entry points

```bash
cargo run -p cli -- --config config/example.toml
```

Flags:

- `--config <PATH>` (repeatable) – load and merge one or more TOML files.

Under the hood `main.rs` loads configuration via `cardinal-config`, constructs a `Cardinal` instance, and invokes `run()`.  The CLI is intentionally thin so integrations can embed the library crate directly when they need finer control.
