# thaum-painter/domain/painter-operations/text

## purpose
Own text-entry and text-stamping semantics for painter authoring.

## owns
- text cell building rules: one glyph char per cell (`text_cells`), ported from
  the old painter's text entry (`buildTextEntryCell` — the document's cells are
  the font; no bitmap rasterization lives here)
- spacing, lead, and newline stepping semantics (spaces/tabs advance without
  placing, newlines stack lines opposite the view's up axis)
- view-plane-relative layout: the view's depth axis never moves, same contract
  as `brush_points`
- text paste behaviors that are still pure operations

## does not own
- clipboard transport
- session cursor ownership
- history

## children-encapsulations
- none

## contents
- `contract.md`
  - text contract
- `text.rs`
  - `text_cells(text, origin, orientation) -> Vec<(CellPoint, char)>`, `TAB_WIDTH`

## dependencies
- `thaum-painter/domain/painter-operations/brush/`
- `thaum-painter/domain/painter-session/clipboard/`

## exposed interfaces
### text layout
send: text, origin cell, camera view orientation
returns: (cell point, glyph char) pairs lying flat in the view plane (depth axis never moves)
effects: none
via: `text_cells`

## interface consumers
- painter-session commands
- tool-state
- future text tool shell in `orchestration/entrypoint/` (after the hotkey/registry seam lands)

## artifacts
- none

## tests
- `text.rs` inline `#[cfg(test)]` module
  - per-char advance along view right, newline stacking below, spaces skip,
    tab width, empty text, side-view (PosX) layout never moves the depth axis

## data
- none

## notes
- Old-system reference: `THAUMWORLD-AUTO-STORY-TELLER/src/ascii_painter/tools.js`
  `buildTextEntryCell` — typing sets the cell's glyph char, clears any graphic
  source, and seeds brush appearance. `PaintedCell` composition (color/weight/
  appearance) belongs to the session bridge when the tool shell lands, exactly
  as it composes brush strokes today.
