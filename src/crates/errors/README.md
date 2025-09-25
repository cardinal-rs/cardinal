# cardinal-errors

Centralised error definitions used across the workspace.

- `CardinalError` is the top-level error type exposed by public APIs.
- `internal` module holds more granular variants (`InternalError::ProviderNotRegistered`, `InvalidWasmModule`, etc.).
- Conversions are implemented so `?` works seamlessly across crate boundaries.

When extending the system, add new variants here rather than inventing ad-hoc error enums in downstream crates.
