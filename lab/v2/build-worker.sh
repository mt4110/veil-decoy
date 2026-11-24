#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
cargo build --locked --release --target wasm32-unknown-unknown --package veil-decoy-edge --target-dir target
wasm-bindgen --target web --no-typescript --out-dir crates/veil-decoy-edge/build \
    target/wasm32-unknown-unknown/release/veil_decoy_edge.wasm
