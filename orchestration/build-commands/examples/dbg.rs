fn main() {
    use thaum_painter_domain::storage::{
        SharedCellPatch, SharedDocumentActionRecord, SharedDocumentFile, SharedDocumentRuntime,
    };
    use thaum_painter_domain::{PaintColor, PaintedCell};
    use thaum_renderer_domain::{CellGraphic, CellPoint};

    let point = CellPoint { x: 0, y: 0, z: 0 };
    let painted = |glyph: char| PaintedCell {
        graphic: CellGraphic::Glyph(glyph),
        color: PaintColor::flat_rgb(255, 255, 255),
        weight_index: 1,
        shader_stack: Vec::new(),
    };
    let at = |runtime: &SharedDocumentRuntime, breath: u32| {
        runtime
            .canvas_for_layer("layer-1", breath)
            .unwrap()
            .get(&point)
            .map(|c| match c.graphic {
                CellGraphic::Glyph(g) => g,
                _ => '?',
            })
            .unwrap_or('_')
    };
    let paint = |runtime: &mut SharedDocumentRuntime, action: &str, block: &str, glyph: char| {
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            action,
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point, None, Some(&painted(glyph)))],
            Some(block.to_string()),
        ));
    };

    // 1. Swap: content must follow the block across the exchanged spans.
    let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
        "doc-1", "Doc", "layer-1", "Layer 1",
    ));
    runtime.split_property_block("layer-1", "raster", "block-1", 8);
    paint(&mut runtime, "a1", "block-1", 'A');
    assert!(runtime.swap_property_blocks("layer-1", "raster", "block-1", "block-2"));
    println!(
        "swap: breath2='{}' (want '_') breath10='{}' (want 'A')",
        at(&runtime, 2),
        at(&runtime, 10)
    );

    // 2. Destructive move into the MIDDLE of a larger bar: the victim splits and
    // both fragments must carry the victim's content.
    let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
        "doc-1", "Doc", "layer-1", "Layer 1",
    ));
    runtime.split_property_block("layer-1", "raster", "block-1", 8);
    runtime.set_property_block_timing_destructive("layer-1", "raster", "block-2", 16, 8);
    paint(&mut runtime, "a1", "block-2", 'B');
    assert!(runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 18, 4));
    for b in &runtime.property_track("layer-1", "raster").unwrap().blocks {
        println!(
            "split: {} start={} len={}",
            b.id, b.start_breath, b.length_breaths
        );
    }
    println!(
        "split: breath17='{}' (want 'B') breath20='{}' (want '_') breath23='{}' (want 'B')",
        at(&runtime, 17),
        at(&runtime, 20),
        at(&runtime, 23),
    );
}
