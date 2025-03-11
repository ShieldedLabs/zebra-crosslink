#!/usr/bin/env bash

set -e

# Run the Zebrad node with the `tokio-console` feature enabled
# enabled and the `tokio_unstable` config flag set.

# Then, open a new terminal and run `tokio-console` (use `cargo install` if you don't have it)

RUSTFLAGS="--cfg tokio_unstable" cargo run \
  --no-default-features \
  --features="tokio-console" \
  --bin zebrad
