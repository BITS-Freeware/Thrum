#!/usr/bin/env bash
# Build the web client's wasm package from the thrum-wasm crate.
# wasm-bindgen-cli must match the wasm-bindgen crate in Cargo.lock (0.2.130).
set -euo pipefail
cd "$(dirname "$0")/.."

rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130
cargo build --release -p thrum-wasm --target wasm32-unknown-unknown
mkdir -p web/src/wasm
wasm-bindgen --target web --out-dir web/src/wasm \
  target/wasm32-unknown-unknown/release/thrum_wasm.wasm
sha256sum web/src/wasm/thrum_wasm_bg.wasm
