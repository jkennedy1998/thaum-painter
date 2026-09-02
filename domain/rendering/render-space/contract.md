# /home/j/Repos/thaum-painter/domain/rendering/render-space

## purpose
Own the render-space handoff that converts thaum-painter file or live state into the scene/state payload consumed by `thaum-renderer`.

## owns
- translation from painter file state into renderer-facing render state
- rules for which saved file fields become direct renderer input
- app-side derived render-space state that exists only to feed the renderer cleanly
- normalization of camera/group/cell/data-lane handoff before renderer boot or frame updates
- the group-to-cell-group mapping: exactly one renderer cell-group per authored group

## does not own
- the saved file manifest itself
- renderer-internal rendering logic after handoff
- editor session history or tool behavior

## children-encapsulations
- none

## contents
- `contract.md`
  - render-space contract
- `render_space.rs`
  - rust group-to-cell-group mapping and handoff assembly, owned by this encapsulation
- `example-render-space-v1.json`
  - first implementation-ready render-space handoff example
- `example-render-space-v1.md`
  - render-space mapping notes
- `schema-render-space-v1.json`
  - first machine-readable render-space schema draft (camera shape is stale, see notes)

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`
- `/home/j/Repos/thaum-renderer/domain/cell/`
- `/home/j/Repos/thaum-renderer/domain/cell-group/`
- `/home/j/Repos/thaum-renderer/domain/composition/`
- `/home/j/Repos/thaum-renderer/domain/coordinate-space/`
- `/home/j/Repos/thaum-renderer/domain/data-lanes/`
- `/home/j/Repos/thaum-renderer/orchestration/boot/`

## exposed interfaces
### build_render_space — assemble the render-space handoff for one manifest at one active breath
send: `&Manifest, active_breath: u32, camera: thaum_renderer_domain::Camera`
returns: `anyhow::Result<RenderSpace>` (`{ camera, composition: thaum_renderer_domain::Composition, data_lanes: thaum_renderer_domain::DataLanes }`)
effects: none
via: rust fn

### build_composition — map a manifest's groups into one `Composition` at one active breath
send: `&Manifest, active_breath: u32`
returns: `anyhow::Result<thaum_renderer_domain::Composition>`
effects: none
via: rust fn

## interface consumers
- future painter app boot flow
- future editor preview/runtime surfaces

## artifacts
- `example-render-space-v1.json`
  - first-pass render-space handoff fixture
- `schema-render-space-v1.json`
  - first-pass machine-readable render-space schema

## tests
- `render_space.rs`'s inline `#[cfg(test)]` module
  - light
  - composites `example-thaum-painter-file-v1.json` into one cell-group per group at several active breaths.

## data
- none

## notes
- this is the seam where the render file or live state sends data to `thaum-renderer`.
- if a field is only needed for storage or authoring, keep it in `domain/file/` and derive renderer-facing state here.
- if a field is directly consumed by the renderer, this boundary should map it through cleanly instead of burying it inside painter-document.
- keep this seam focused on handoff assembly, not long-lived saved truth and not live editor policy.
- if render-space starts absorbing too many concerns, split it into child seams such as scene assembly, timing/data-lane mapping, preview-only derivation, or visible group resolution.
- one authored `group` in, one `cell_groups[]` entry out — direct 1:1, no compositing step. See `/home/j/Repos/thaum-painter/context/module-concept-audit.md`'s "superseded" section: the earlier module-wrapper/compositing shape was reversed because the renderer's camera is universal, not module-local, so there's no longer a reason to bundle several layers before handoff. Renderer's own `domain/composition/overlap-policy` and `pass-order` already resolve inter-group overlap/ordering, so painter doesn't need a duplicate compositing pass.
- implemented in Rust, matching `domain/file/manifest/manifest.rs`'s conventions: `build_render_space`/`build_composition` build real `thaum-renderer-domain` types (`Camera`, `CellGroup`, `Composition`, `DataLanes`) in-process rather than the JSON handoff shape, since the live-preview requirement means the renderer is embedded in the same process (see `context/roadmap.md`).
- breath resolution: a group contributes cells only if it is `visible` and has a `raster_segments[]` entry whose `[start_breath, end_breath]` window contains the active breath; a `move`-kind property's active block (same window rule) adds an offset to the group's `local_placement` before its voxels are placed. This matched `example-render-space-v1.json` exactly on first implementation, including breath 4's dim-to-bright glow segment swap plus its move-block shift.
- **known gap:** `schema-render-space-v1.json`/`example-render-space-v1.json`'s `camera` shape (`orientation`/`focus_plane`/`viewport_scale`/`target_world`) does not match `thaum-renderer-domain`'s actual `Camera` struct (`position`/`focus_target`/`swing`/`roll`/`projection_mode`/`zoom`/`visible_plane_radius`). `build_render_space` sidesteps this by taking an already-resolved `Camera` as a parameter rather than mapping it itself — that mapping is `domain/rendering/camera/`'s job (still unimplemented). The JSON schema/example need a pass once that mapping is designed.
- this was written with a working `cargo`/`rustc` (fixed in this session) and is compiler-verified: `cargo test -p thaum-painter-domain` passes (22 tests total).
