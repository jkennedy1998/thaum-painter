// Headless repro of the main.rs paint flow: press -> stage -> release/commit -> render.
use std::collections::BTreeSet;
use thaum_renderer_domain::{
    camera_view_orientation_for_camera, CameraRoll, CameraSwing, CameraViewOrientation, CellPoint,
};

fn flat_view() -> CameraViewOrientation {
    camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0)
}
use thaum_painter_domain::{
    append_action_record, load_or_create_shared_document, Canvas, CanvasBounds, CanvasPlaneAxis,
    PaintHand, PaintTarget, PaintTool, PainterSelection, SharedCellPatch,
    SharedDocumentActionRecord, SharedDocumentFile, SharedDocumentPaths, SharedDocumentRuntime,
    ToolState,
};

fn collect_canvas_patches(before: &Canvas, after: &Canvas) -> Vec<SharedCellPatch> {
    let positions: BTreeSet<_> = before.keys().chain(after.keys()).copied().collect();
    positions
        .into_iter()
        .filter_map(|position| {
            let b = before.get(&position);
            let a = after.get(&position);
            if b == a {
                None
            } else {
                Some(SharedCellPatch::new(position, b, a))
            }
        })
        .collect()
}

#[test]
fn paint_flow_end_to_end_on_fresh_document() {
    let dir = std::env::temp_dir().join(format!("painter-repro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = SharedDocumentPaths::new(dir.clone());
    let mut runtime = load_or_create_shared_document(
        &paths,
        SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1"),
    )
    .unwrap();

    // Boot canvas init (main.rs line ~1349)
    let mut canvas = runtime
        .canvas_for_layer("layer-1", 0)
        .cloned()
        .unwrap_or_default();
    println!("boot canvas cells: {}", canvas.len());

    let bounds = CanvasBounds {
        x0: -10,
        y0: -8,
        x1: 10,
        y1: 8,
        z: 0,
        plane_axis: CanvasPlaneAxis::Z,
    };
    let mut selection = PainterSelection::new(bounds);
    let mut tool_state = ToolState::default();
    tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Brush);
    println!(
        "left target: {:?}",
        tool_state.hand_state(PaintHand::Left).target
    );
    assert_eq!(
        tool_state.hand_state(PaintHand::Left).target,
        PaintTarget::Image
    );

    // PRESS: block resolution
    let block_id = runtime
        .active_raster_block_id("layer-1", 0)
        .expect("block covering breath 0");
    println!("block id: {block_id}");

    // PRESS: stage one chunk
    let before = canvas.clone();
    let mut candidate = before.clone();
    tool_state.apply_at_for_hand(
        &mut candidate,
        &mut selection,
        CellPoint { x: 0, y: 0, z: 0 },
        PaintHand::Left,
        bounds,
        flat_view(),
    );
    let patches = collect_canvas_patches(&before, &candidate);
    println!("press patches: {}", patches.len());
    assert!(!patches.is_empty(), "brush press produced no patches");
    runtime.stage_canvas_patches("layer-1", &block_id, &patches);
    canvas = candidate;

    // Per-frame render check: does the staged cell show up in composition groups?
    let canvas_view = runtime
        .canvas_for_layer("layer-1", 0)
        .expect("canvas_for_layer after stage");
    println!("runtime canvas cells after stage: {}", canvas_view.len());
    assert!(
        canvas_view.contains_key(&CellPoint { x: 0, y: 0, z: 0 }),
        "staged cell invisible in render path"
    );

    // RELEASE: commit
    let start_canvas = before;
    let patches = collect_canvas_patches(&start_canvas, &canvas);
    assert!(!patches.is_empty());
    let record = SharedDocumentActionRecord::cell_patch_set(
        "1",
        "doc-1",
        "layer-1",
        "tester",
        "2026-09-02T00:00:00Z",
        patches,
        Some(block_id.clone()),
    );
    append_action_record(&paths.actions_file_path, &record).unwrap();
    runtime.apply_action_record(record);

    // Next frame render
    let canvas_view = runtime.canvas_for_layer("layer-1", 0).unwrap();
    assert!(canvas_view.contains_key(&CellPoint { x: 0, y: 0, z: 0 }));

    // Undo (KeyZ path): undo_top_action -> revert record
    if let Some(revert) = runtime.undo_top_action("layer-1") {
        let rec = SharedDocumentActionRecord::revert_patch_set(
            "2",
            "doc-1",
            "layer-1",
            "tester",
            "2026-09-02T00:00:01Z",
            revert.patches,
            Some(revert.block_id),
            revert.action_id,
        );
        append_action_record(&paths.actions_file_path, &rec).unwrap();
        runtime.push_history_record(rec);
    }
    let canvas_view = runtime.canvas_for_layer("layer-1", 0).unwrap();
    assert!(
        !canvas_view.contains_key(&CellPoint { x: 0, y: 0, z: 0 }),
        "undo did not revert cell"
    );
}

#[test]
fn paint_flow_on_real_local_document() {
    // Mirror the fixed boot: seek the playhead to a breath the raster track covers
    // (the document has a leading gap at breaths 0..8).
    let root = std::path::Path::new("../orchestration/artifacts/shared-documents/local-document");
    if !root.exists() {
        panic!("local-document not found at {:?}", root);
    }
    let paths = SharedDocumentPaths::new(root.to_path_buf());
    let document =
        thaum_painter_domain::storage::load_document_file(&paths.document_file_path).unwrap();
    let actions =
        thaum_painter_domain::storage::load_action_records(&paths.actions_file_path).unwrap();
    println!("replayed action records: {}", actions.len());
    let mut runtime = SharedDocumentRuntime::replay(document, actions);
    println!("runtime actions: {}", runtime.actions.len());

    let breath = runtime
        .first_breath_with_raster_block("layer-1")
        .expect("raster track exists");
    println!("boot playhead seeks breath {breath}");

    let mut canvas = runtime
        .canvas_for_layer("layer-1", breath)
        .cloned()
        .unwrap_or_default();
    println!("boot canvas cells at breath {breath}: {}", canvas.len());

    let bounds = CanvasBounds {
        x0: -60,
        y0: -50,
        x1: -40,
        y1: -30,
        z: -12,
        plane_axis: CanvasPlaneAxis::Z,
    };
    let mut selection = PainterSelection::new(bounds);
    let mut tool_state = ToolState::default();
    tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Brush);

    let block_id = runtime
        .active_raster_block_id("layer-1", breath)
        .expect("block covering the sought breath");
    println!("block id: {block_id}");

    // Press with no selection: the stroke must produce patches.
    let before = canvas.clone();
    let mut candidate = before.clone();
    tool_state.apply_at_for_hand(
        &mut candidate,
        &mut selection,
        CellPoint {
            x: -50,
            y: -40,
            z: -12,
        },
        PaintHand::Left,
        bounds,
        flat_view(),
    );
    let patches = collect_canvas_patches(&before, &candidate);
    println!("press patches (no selection): {}", patches.len());
    assert!(
        !patches.is_empty(),
        "brush press produced no patches with empty selection"
    );
    runtime.stage_canvas_patches("layer-1", &block_id, &patches);
    canvas = candidate;

    let view = runtime
        .canvas_for_layer("layer-1", breath)
        .expect("canvas after stage");
    assert!(
        view.contains_key(&CellPoint {
            x: -50,
            y: -40,
            z: -12
        }),
        "staged cell invisible"
    );

    // Press with a selection elsewhere on the plane: edits outside the selection
    // are gated. This is expected selection semantics, surfaced here as a probe.
    let mut selection = PainterSelection::new(bounds);
    selection.apply_plane_points([CellPoint {
        x: -60,
        y: -50,
        z: -12,
    }]);
    let before = canvas.clone();
    let mut candidate = before.clone();
    tool_state.apply_at_for_hand(
        &mut candidate,
        &mut selection,
        CellPoint {
            x: -55,
            y: -45,
            z: -12,
        },
        PaintHand::Left,
        bounds,
        flat_view(),
    );
    let patches = collect_canvas_patches(&before, &candidate);
    println!("press patches (selection elsewhere): {}", patches.len());
}
