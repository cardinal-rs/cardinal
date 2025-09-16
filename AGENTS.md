# Repository Guidelines

## Project Structure & Module Organization
This workspace is driven by `Cargo.toml` at the root and groups crates under `src/crates`. `base` holds shared async services and provider abstractions, `cli` exposes the `cardinal` binary in `src/main.rs`, while `cardinal-config` and `cardinal-errors` centralize configuration loading and error types. Keep new crates inside `src/crates/<name>` with their own `Cargo.toml`, and colocate assets or fixtures in each crate to keep ownership clear.

## Build, Test, and Development Commands
Use `cargo build` for a full debug build and `cargo check` for fast compilation without binaries. Run `cargo test` to execute unit and integration suites across all crates. The CLI can be exercised with `cargo run -p cli -- --help` or by passing scenario-specific flags. Format code with `cargo fmt` and lint with `cargo clippy -- -D warnings` before sending changes.

## Coding Style & Naming Conventions
Follow Rust 2021 defaults: four-space indentation, `snake_case` for modules/functions, and `PascalCase` for types and traits. Keep modules small and favor async traits already used in `base`. Do not hand-edit formatting—run `cargo fmt` so the workspace stays consistent. Centralize shared behaviors in `base` and prefer re-exporting from `lib.rs` for discoverability.

## Testing Guidelines
Write focused unit tests inside `#[cfg(test)]` modules next to the code, and integration tests under `src/crates/<name>/tests` when they span multiple modules. Tests should use descriptive snake_case names such as `loads_default_config`. Async scenarios should leverage `#[tokio::test]` since Tokio is already a workspace dependency. Aim to cover new public APIs and document expected tracing output when helpful.

## Commit & Pull Request Guidelines
The current history uses short imperative messages (e.g., `Initial commit`); continue that format, keeping the summary under 72 characters and expanding details in the body when needed. Each pull request should describe intent, list affected crates, link any tracking issues, and attach relevant command output (e.g., `cargo test`). Confirm formatting and linting checks have been run before requesting review.

## Configuration Notes
Configuration helpers live in `cardinal-config`; prefer extending its APIs rather than reading files ad hoc. Surface new options through strongly typed structs and document defaults in `config.rs` so the CLI crate can validate input consistently.
