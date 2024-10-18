#!/usr/bin/env bash

# The script runs almost all CI checks locally.
#
# Requires installed:
# - Rust `1.81.0`
# - Nightly rust formatter
# - `cargo install cargo-sort`

cargo +nightly fmt --all -- --check &&
cargo sort -w --check &&
cargo clippy --all-targets --all-features &&
cargo test --all-features --workspace