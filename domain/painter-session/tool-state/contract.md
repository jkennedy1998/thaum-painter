# /home/j/Repos/thaum-painter/domain/painter-session/tool-state

## purpose
Own live tool settings and active authoring-hand state for the painter session.

## owns
- active tool selections
- left/right hand brush state
- left/right hand color, glyph, weight, brush-size, and fill-connectivity source-of-truth state
- the rule that hand color can be either direct RGB or a material-backed choice
- edit-channel masks
- per-hand image-versus-selection target state
- per-hand channel locks for selector updates
- per-tool tool-specific property declarations (`PaintTool::property_row_ids`) so property panels can hide rows neither equipped tool uses
- text-entry and paste option state that belongs to the live session

## does not own
- persistent file manifest ownership
- pure tool operations
- history ownership

## children-encapsulations
- none

## contents
- `contract.md`
  - tool-state contract
- `tool_state.rs`
  - live left/right hand tool-state, selection-gated image editing, and masked paint application, including blank-glyph preservation on empty cells

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-operations/`
- `/home/j/Repos/thaum-painter/domain/painter-session/paint-color/`
- `/home/j/Repos/thaum-painter/domain/file/storage/`

## exposed interfaces
- none

## interface consumers
- painter-session

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `tool_state.rs`
  - light
  - validates per-hand tool selection, shared brush/erase properties, selection-gated image editing, fill masking, channel locks, and the per-tool property-row manifest

## data
- none

## notes
- source-of-truth from J: left and right mouse buttons should map to separately assignable painter tools, matching the old painter's hand-based workflow.
- source-of-truth from J: the color and weight selected for the left and right mouse button should live as a single source of truth and support tool-side shenanigans.
- this now stores painter-facing color choice through `paint-color/` so flat sprite colors and material palettes share one hand-state seam.
- source-of-truth from J: left/right masking between image editing and selection editing was useful and should come back.
- source-of-truth from J: once a selection exists, image-editing tools should have one common seam that limits edits to the selected cells so brush, erase, fill, rect tools, and future delete actions all obey the same rule.
- source-of-truth from J: left/right channel locks for character, weight, or color were useful for in-depth illustration where only some layers should change.
- source-of-truth from J: tools use properties; tools that share the same property UI reuse the same panel pieces, all tool properties stay tracked per tool, and the properties panel hides rows neither the left nor right hand's tool uses so panels stay thin as tools gain properties.
- when graphic editing is disabled and the user edits an empty cell, painter now preserves that cell's implicit blank glyph (`' '`) rather than silently swapping in the hand's current graphic.
