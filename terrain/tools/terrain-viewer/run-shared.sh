#!/bin/sh
set -eu
database="${1:-}"
if [ -n "$database" ]; then
    case "$database" in /*) ;; *) database="$PWD/$database";; esac
fi
cd "$(dirname "$0")/../.."
if [ -z "$database" ]; then
    database="tools/terrain-viewer/terrain-shared.sqlite3"
    if [ ! -f "$database" ]; then database="build/national-shared-z6-z13-v1/terrain-shared.sqlite3"; fi
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/build/terrain-viewer-target}"
cargo build --locked --release --manifest-path tools/terrain-viewer/Cargo.toml --bin terrain-shared-viewer
exec "$CARGO_TARGET_DIR/release/terrain-shared-viewer" --database "$database"
