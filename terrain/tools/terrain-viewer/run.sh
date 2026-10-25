#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
database="${1:-build/terrain-national-xyz-20261024/terrain.sqlite3}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/build/terrain-viewer-target}"
cargo build --locked --release --manifest-path tools/terrain-viewer/Cargo.toml
exec "$CARGO_TARGET_DIR/release/terrain-viewer" "$database"
