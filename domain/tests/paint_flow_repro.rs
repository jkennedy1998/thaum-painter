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
    SharedDocumentActionRecord, SharedDocumentFile, SharedDocumentPaths, ToolState,
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
    // The real local-document artifact is still a pre-bars schema-v1 file. The
    // binary-bars schema gate rejects it cleanly at load (typed error, no partial
    // state) — J migrates real files case-by-case, so the repro against it now
    // asserts that clean rejection instead of a paint flow.
    let root = std::path::Path::new("../orchestration/artifacts/shared-documents/local-document");
    if !root.exists() {
        panic!("local-document not found at {:?}", root);
    }
    let paths = SharedDocumentPaths::new(root.to_path_buf());
    let error = thaum_painter_domain::storage::load_document_file(&paths.document_file_path)
        .expect_err("v1 local document must reject cleanly under the v2 gate");
    let message = error.to_string();
    assert!(message.contains("schema v1"), "unexpected error: {message}");
    assert!(message.contains("reads v2"), "unexpected error: {message}");
    let _ = paths;
}
