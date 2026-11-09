### 2026-11-09-detailer-typeface
- j-quote: "No typeface swap during active stroke that is bloat. Was mostly just trying to understand the tool. This is good if it's per type that's exactly what I need!"
- interpretation: A detail stroke remains typeface-local; no special active-stroke typeface-change behavior is added.

### 2026-11-09-detailer-preview-parity
- j-quote: "preview is not the same implementation as the final. i see a difference sometimes when i puck up. erase preview is actually opposite too, so thats bad."
- interpretation: The detailer preview must render exactly what release will paint — the preview replaces the cell (document pass suppresses the pre-edit cell on the active layer), never composites over it.

### 2026-11-09-detailer-size
- j-quote: "i need size to be availible to use for this detailer tool. ideally what happens is it just authors a thicker subpixel line so that the directional buildup can happen faster."
- interpretation: The Detailer exposes the hand's brush_size row; the size maps down into the subcell buffer as a thicker footprint so buildup lands in fewer passes.

### 2026-11-09-detailer-size-shapes
- j-quote: "1 will be 1 subpixel. 2 will be 2x2 subpixel. 3 will be 3x3 subpixel. 4 will be 4x4 subpixels with the corders removed. 5x5 with corners renmoved for the size 5."
- interpretation: Detailer brush sizes 1-3 author solid NxN subpixel blocks; sizes 4-5 author the block minus its four corners.

### 2026-11-09-detailer-fluid-corners
- j-quote: "if i press the corder of a cell it should draw 1 cell at each appropriate corner of a cell. we dont show the user this directly, but thats the system behavior. it shouldnt snap, it should really feel fluid like just drawing over multiple cells simply with this detailer tool, so a user can turn their brain off and just additivly draw onto the canvas."
- interpretation: The detailer footprint anchors at the pointer's continuous position and decomposes across every cell it touches (corner presses author corner subpixels in all adjacent cells); nothing snaps to the pointer's cell.
