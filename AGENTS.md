# Repository Guidelines

## Project Structure & Module Organization
- Workspace managed by the root `Cargo.toml`; crates live under `src/crates/` (e.g., `cardinal`, `plugins`, `proxy`).
- Shared assets and WASM fixtures reside in `tests/wasm-plugins/`; each scenario keeps its own `plugin.wasm` and JSON fixtures.
- Middleware infrastructure sits in `src/crates/plugins/`; WASM runtime support is in `src/crates/wasm-plugins/`.

## Build, Test, and Development Commands
- `cargo check` — fast type-check across the entire workspace.
- `cargo test` — run unit/integration tests for all crates.
- `cargo test -p cardinal-plugins` — focus on middleware registry & runner tests.
- `npx asc plugin.ts -o plugin.wasm --optimize --exportRuntime` — rebuild AssemblyScript fixtures inside `tests/wasm-plugins/<case>/`.

## Coding Style & Naming Conventions
- Rust 2021 edition; four-space indentation, `snake_case` for modules/functions, `PascalCase` for types/traits.
- Format with `cargo fmt`; lint with `cargo clippy -- -D warnings` before submission.
- Middleware identifiers should remain consistent across config, registry, and runtime (e.g., `restricted_route_middleware`).

## Testing Guidelines
- Prefer unit tests alongside implementation modules (`#[cfg(test)]` blocks).
- WASM scenarios live under `tests/wasm-plugins/<case>/` with `incoming_request.json` and `expected_response.json` fixtures.
- Execute middleware runtime tests with `cargo test -p cardinal-plugins`; WASM coverage via `cargo test -p cardinal-wasm-plugins`.

## Commit & Pull Request Guidelines
- Follow the existing short imperative commit style (`Add WASM runner pipeline`, `Fix plugin lookup`). Keep summaries under ~72 characters.
- PRs should explain the change scope, affected crates, and include relevant command output (e.g., `cargo test`). Link issues or specs when applicable.
- Ensure formatters and linting have been run; attach screenshots only when UX or CLI output changes.

## Security & Configuration Tips
- Treat WASM modules as untrusted input; validate exports through `WasmPlugin::new` before wiring into pipelines.
- Protect secrets by keeping environment-specific configuration outside the repo; rely on typed structs in `cardinal-config` for runtime configuration.
