# thaum-painter file v3

V3 makes a saved raster cell a complete portable source appearance rather than a glyph plus RGB shortcut.

## Voxel appearance

Each `voxel` has its grid `x`, `y`, and `z` plus an `appearance`:

- `graphic`: `{ "kind": "glyph", "glyph": "A" }` or `{ "kind": "sprite", "asset_file": "cell-sprites/name.png" }`
- `color`: a flat RGB value, a material `asset_file`, or `slots` with `a`, `b`, and `c` flat/material assignments
- `weight_index`
- ordered `shader_stack` asset filenames

Asset filenames are non-empty paths relative to the consuming renderer asset root. They are never absolute paths or game-local IDs. Empty cells are absent from the voxel list.

## Layer placement and origin

`placement` is the authored board location. `origin` is the fixed pivot for later rotation and is not changed by animated `move` properties.

## Flat export preset

`document.exports.flat` is optional (`null` when absent) declarative configuration for the future general asset exporter. It records the output filename, display name, facing, and `"interpolation": "preserve"`. It does not record output folders, timestamps, game IDs, or exporter execution results.

## Generation policy

V3 is a breaking generation. V1 and V2 are deliberately rejected; painter does not migrate old portable files.
