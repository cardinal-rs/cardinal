install-tools:
    rustup component add clippy
    rustup component add rustfmt

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --all-features -- -D warnings

cargo-test:
    just ubuntu-essentials
    cargo test --all-features

ubuntu-essentials:
    sudo apt-get update
    sudo apt-get install -y \
      build-essential pkg-config \
      libssl-dev zlib1g-dev \
      clang libclang-dev \
      libunwind-dev
