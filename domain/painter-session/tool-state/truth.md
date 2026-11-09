### 2026-11-09-detailer-typeface
- j-quote: "No typeface swap during active stroke that is bloat. Was mostly just trying to understand the tool. This is good if it's per type that's exactly what I need!"
- interpretation: A detail stroke remains typeface-local; no special active-stroke typeface-change behavior is added.

### 2026-11-09-detailer-preview-parity
- j-quote: "preview is not the same implementation as the final. i see a difference sometimes when i puck up. erase preview is actually opposite too, so thats bad."
- interpretation: The detailer preview must render exactly what release will paint — the preview replaces the cell (document pass suppresses the pre-edit cell on the active layer), never composites over it.

### 2026-11-09-detailer-size
- j-quote: "i need size to be availible to use for this detailer tool. ideally what happens is it just authors a thicker subpixel line so that the directional buildup can happen faster."
- interpretation: The Detailer exposes the hand's brush_size row; the size maps down into the subcell buffer as a thicker footprint so buildup lands in fewer passes.
