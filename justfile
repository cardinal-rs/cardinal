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

cargo-czip-test:
    wasm-pack test --node src/crates/czip

ubuntu-essentials:
    sudo apt-get update
    sudo apt-get install -y \
      build-essential pkg-config \
      libssl-dev zlib1g-dev \
      clang libclang-dev \
      libunwind-dev \
      llvm \
      lld \
      binutils \
      libgcc-12-dev \
      libstdc++-12-dev
