install-tools:
    rustup component add clippy
    rustup component add rustfmt

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-features -- -D warnings

cargo-test:
    cargo test --all-features
