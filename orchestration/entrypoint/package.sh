#!/usr/bin/env bash
# Packages a self-contained thaum-painter distribution folder: the release
# binary + the renderer's asset folder + both license files. Output lands in
# artifacts/release/ (git-ignored) as a staged folder and a .tar.gz — builds
# are distribution artifacts, never committed to the repo.
#
# Usage: ./package.sh [platform-slug]   (default: lowercase uname, e.g. "linux")
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
painter_root="$(cd "$script_dir/../.." && pwd)"
renderer_root="$(cd "$painter_root/../thaum-renderer" && pwd)"
artifacts_dir="$script_dir/artifacts"
platform_slug="${1:-$(uname -s | tr '[:upper:]' '[:lower:]')}"

version="$(grep -m1 '^version' "$script_dir/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
stage_name="thaum-painter-$version-$platform_slug"
stage_dir="$artifacts_dir/release/$stage_name"

export CARGO_TARGET_DIR="$artifacts_dir/target"
source "$HOME/.cargo/env" 2>/dev/null || true

cd "$painter_root"
cargo build --release -p thaum-painter-entrypoint

rm -rf "$stage_dir"
mkdir -p "$stage_dir"
cp "$CARGO_TARGET_DIR/release/thaum-painter-entrypoint" "$stage_dir/thaum-painter"
cp -r "$renderer_root/orchestration/renderer-assets" "$stage_dir/renderer-assets"
cp "$painter_root/context/LICENSE.md" "$stage_dir/LICENSE"
cp "$renderer_root/context/LICENSE.md" "$stage_dir/RENDERER-LICENSE"

tar -czf "$artifacts_dir/release/$stage_name.tar.gz" -C "$artifacts_dir/release" "$stage_name"
echo "packaged: $artifacts_dir/release/$stage_name.tar.gz"
echo "staged folder kept for inspection: $stage_dir"
