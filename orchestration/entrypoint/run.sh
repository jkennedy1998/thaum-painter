#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
painter_root="$(cd "$script_dir/../.." && pwd)"
artifacts_dir="$script_dir/artifacts"
mkdir -p "$artifacts_dir"

export CARGO_TARGET_DIR="$artifacts_dir/target"
source "$HOME/.cargo/env" 2>/dev/null || true

cd "$painter_root"
cargo build --release -p thaum-painter-entrypoint

exec "$CARGO_TARGET_DIR/release/thaum-painter-entrypoint"
