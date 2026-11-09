# thaum-painter/domain/modules/individuals/detailer-grid

## purpose
Own the visible enlarged, typeface-derived comparison grid through which a painter edits one active detailer stroke.

## owns
- detail-grid presentation of the active comparison tile's grayscale alpha coverage
- pointer-to-comparison-pixel mapping within the grid's visibly rendered bounds
- detail-grid pointer capture and forwarding pixel edits/release to the shared detailer session

## does not own
- the detail stroke buffer, resolved glyph, or pending commit truth, owned by `painter-session/tool-state/`
- selecting a canvas target, document writes, or canvas preview composition, owned by `painter-session/canvas-pointer/`
- generic module chrome, pointer dispatch, or persistence primitives, owned by `thaum-renderer/domain/modules/`

## children-encapsulations
- none

## contents
- `contract.md`
  - detail-grid boundary contract
- `detailer_grid_module.rs`
  - `DetailerGridModule` UI, grid mapping, and pointer forwarding

## dependencies
- `thaum-painter/domain/painter-session/tool-state/`: `DetailerSession`, `DetailerStroke`, `PaintHand`
- `thaum-renderer/domain/modules/`: `Module`, chrome, pointer events, and palette types

## exposed interfaces
### DetailerGridModule::new — bind an enlarged comparison grid to one live detail session
send: { id: string, rect: ModuleRect, session: Rc<RefCell<DetailerSession>>, resolve: Rc<Fn(&DetailerStroke) -> Option<char>> }
returns: { module: DetailerGridModule }
effects: none
via: rpc `DetailerGridModule::new`

## interface consumers
- `thaum-painter/orchestration/entrypoint/`: registers the painter-bound grid and supplies the live typeface resolver

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `detailer_grid_module.rs`
  - light
  - validates typeface-derived pixel mapping, captured drag forwarding, release-to-one-commit, and safe wrong-hand/out-of-grid/no-target no-ops

## data
- none

## notes
- Each comparison pixel is enlarged by a fixed UI scale only; the grid dimensions always come from the active stroke's typeface-derived tile and never from legacy glyph constants.
- Alpha coverage uses an ASCII-safe four-level `.`/`:`/`o`/`#` display so a proportional selected typeface cannot make the detail controls illegible.
