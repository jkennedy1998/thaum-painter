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

# stderr/stdout go to a rolling log so crashes leave a trace even when the
# app was launched from a desktop entry with no terminal attached.
log_file="$artifacts_dir/logs/painter-$(date +%Y%m%d).log"
mkdir -p "$(dirname "$log_file")"
exec "$CARGO_TARGET_DIR/release/thaum-painter-entrypoint" >>"$log_file" 2>&1
