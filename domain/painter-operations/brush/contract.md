# /home/j/Repos/thaum-painter/domain/painter-operations/brush

## purpose
Own direct brush-style cell application and erase semantics.

## owns
- single-cell draw semantics
- erase semantics
- brush-to-cell mapping rules including color, weight, and renderable appearance payloads
- camera-facing brush footprints: `brush_points(position, size, orientation)` expands the tip along the camera view orientation's right/up axes (`CameraViewOrientation` from the renderer domain) so the tip always lies flat in the plane the camera is looking at, at every swing and roll. The default PosZ view reproduces the original x/y expansion. Flood fill stays canvas-topology based and is angle-independent by construction.

## does not own
- history
- selection ownership
- renderer handoff

## children-encapsulations
- none

## contents
- `contract.md`
  - brush contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-document/`

## exposed interfaces
- none

## interface consumers
- painter-session commands
- shape/text/fill operations

## artifacts
- none

## tests
- `brush.rs` inline `#[cfg(test)]` module
  - footprint size/shape in the default view, depth axis fixed at every swing (PosX: x never moves, z/y spread)

## data
- none
