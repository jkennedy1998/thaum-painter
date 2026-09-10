//! App-side camera→plane mapping and viewport-scroll intent: how the live
//! paint viewport maps onto the active canvas plane, and how HUD/drawing-space
//! scroll gestures move the renderer camera.

use std::cell::RefCell;
use std::rc::Rc;

use thaum_renderer_domain::{
    active_depth_axis_for_swing, camera_view_orientation_for_camera,
    remap_camera_units_to_active_plane_world, unproject_view_relative_to_world, Camera, ModuleRect,
    ViewRelativePoint, WorldAxis, WorldPoint,
};

use crate::fill::CanvasBounds;
use crate::fill::CanvasPlaneAxis;
use crate::paint_canvas_bounds_module::{DrawingSpaceWheelMode, PaintCanvasBoundsModule};
use crate::selection_state::PainterSelection;

/// Where the live paint viewport starts on the shared HUD layer before the
/// user moves or resizes it through painter's bounds gizmo.
pub const INITIAL_PAINT_CANVAS_VIEWPORT: ModuleRect = ModuleRect {
    x0: -24,
    y0: -8,
    x1: -9,
    y1: 8,
};

pub const INITIAL_PAINT_CANVAS_BOUNDS: CanvasBounds = CanvasBounds {
    x0: -23,
    y0: -7,
    x1: -10,
    y1: 6,
    z: 0,
    plane_axis: CanvasPlaneAxis::Z,
};

pub fn canvas_plane_axis_for_camera(camera: &Camera) -> CanvasPlaneAxis {
    match active_depth_axis_for_swing(camera.swing) {
        WorldAxis::X => CanvasPlaneAxis::X,
        WorldAxis::Y => CanvasPlaneAxis::Y,
        WorldAxis::Z => CanvasPlaneAxis::Z,
    }
}

pub fn active_plane_coordinate_for_camera(camera: &Camera) -> i32 {
    match active_depth_axis_for_swing(camera.swing) {
        WorldAxis::X => camera.focus_target.x,
        WorldAxis::Y => camera.focus_target.y,
        WorldAxis::Z => camera.focus_target.z,
    }
}

pub fn plane_coordinates(bounds: CanvasBounds, point: WorldPoint) -> (i32, i32) {
    match bounds.plane_axis {
        CanvasPlaneAxis::Z => (point.x, point.y),
        CanvasPlaneAxis::X => (point.y, point.z),
        CanvasPlaneAxis::Y => (point.x, point.z),
    }
}

pub fn offset_rect(rect: ModuleRect, dx: i32, dy: i32) -> ModuleRect {
    ModuleRect {
        x0: rect.x0 + dx,
        y0: rect.y0 + dy,
        x1: rect.x1 + dx,
        y1: rect.y1 + dy,
    }
}

pub fn viewport_content_rect_in_camera_units(viewport: ModuleRect, camera: &Camera) -> ModuleRect {
    offset_rect(
        PaintCanvasBoundsModule::content_rect(viewport),
        camera.hud_pan_offset.x,
        camera.hud_pan_offset.y,
    )
}

pub fn viewport_content_center_in_camera_units(viewport: ModuleRect, camera: &Camera) -> [f32; 2] {
    let content = viewport_content_rect_in_camera_units(viewport, camera);
    [
        (content.x0 + content.x1) as f32 * 0.5,
        (content.y0 + content.y1) as f32 * 0.5,
    ]
}

pub fn focus_target_for_world_at_camera_units(
    camera: Camera,
    world: WorldPoint,
    camera_units: [f32; 2],
) -> WorldPoint {
    let orientation = camera_view_orientation_for_camera(camera.swing, camera.roll);
    let offset = unproject_view_relative_to_world(
        orientation,
        WorldPoint::origin(),
        ViewRelativePoint {
            right: camera_units[0].round() as i32,
            up: camera_units[1].round() as i32,
            depth: 0,
        },
    );
    WorldPoint {
        x: world.x - offset.x,
        y: world.y - offset.y,
        z: world.z - offset.z,
    }
}

/// Swings/rolls the camera while keeping the world point under the viewport
/// center pinned to the viewport center.
pub fn reorient_camera_around_viewport_center(
    camera: &mut Camera,
    viewport: ModuleRect,
    reorient: impl FnOnce(&mut Camera),
) {
    let center = viewport_content_center_in_camera_units(viewport, camera);
    let anchor = remap_camera_units_to_active_plane_world(*camera, center);
    let mut next = *camera;
    reorient(&mut next);
    next.focus_target = focus_target_for_world_at_camera_units(next, anchor, center);
    *camera = next;
}

pub fn canvas_bounds_for_viewport(
    viewport: ModuleRect,
    camera: &Camera,
    move_offset: WorldPoint,
) -> CanvasBounds {
    let content = viewport_content_rect_in_camera_units(viewport, camera);
    let mut bounds = CanvasBounds {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
        z: active_plane_coordinate_for_camera(camera),
        plane_axis: canvas_plane_axis_for_camera(camera),
    };
    // The visible drawing-space rect lives in RENDER space, but the input gate
    // (and every consumer of these bounds — selection, fill, tool strokes)
    // works in DOCUMENT space, where positions have had the active layer's
    // move offset subtracted (J 2026-09-10: the gate used to compare a
    // document point against render-space bounds, so a layer with a non-
    // default move shifted the on-screen input region by the move — and a
    // depth move took the corrected position off the bounds' plane entirely,
    // blocking input). Subtracting the move here anchors the bounds to the
    // document, so the accepted region is exactly the visible rect.
    let to_document = |world: WorldPoint| WorldPoint {
        x: world.x - move_offset.x,
        y: world.y - move_offset.y,
        z: world.z - move_offset.z,
    };
    let lower_left = to_document(remap_camera_units_to_active_plane_world(
        *camera,
        [content.x0 as f32, content.y0 as f32],
    ));
    let upper_right = to_document(remap_camera_units_to_active_plane_world(
        *camera,
        [content.x1 as f32, content.y1 as f32],
    ));
    let (x0, y0) = plane_coordinates(bounds, lower_left);
    let (x1, y1) = plane_coordinates(bounds, upper_right);
    bounds.x0 = x0.min(x1);
    bounds.y0 = y0.min(y1);
    bounds.x1 = x0.max(x1);
    bounds.y1 = y0.max(y1);
    bounds.z = match bounds.plane_axis {
        CanvasPlaneAxis::Z => lower_left.z,
        CanvasPlaneAxis::X => lower_left.x,
        CanvasPlaneAxis::Y => lower_left.y,
    };
    bounds
}

pub fn sync_canvas_bounds_to_camera(
    viewport: &Rc<RefCell<ModuleRect>>,
    bounds: &Rc<RefCell<CanvasBounds>>,
    selection: &Rc<RefCell<PainterSelection>>,
    camera: &Camera,
    move_offset: WorldPoint,
) {
    let next = canvas_bounds_for_viewport(*viewport.borrow(), camera, move_offset);
    *bounds.borrow_mut() = next;
    selection.borrow_mut().set_plane_bounds(next);
}

pub fn scroll_step_count(delta: f32) -> i32 {
    if delta == 0.0 {
        0
    } else {
        let rounded = delta.round() as i32;
        if rounded == 0 {
            delta.signum() as i32
        } else {
            rounded
        }
    }
}

pub fn pan_hud_and_focus_right(camera: &mut Camera, delta: i32) {
    camera.pan_hud_right(delta);
    camera.pan_focus_right(-delta);
}

pub fn pan_hud_and_focus_up(camera: &mut Camera, delta: i32) {
    camera.pan_hud_up(delta);
    camera.pan_focus_up(-delta);
}

pub fn apply_hud_scroll(camera: &mut Camera, delta_x: f32, delta_y: f32) {
    let x_steps = scroll_step_count(delta_x);
    let y_steps = scroll_step_count(delta_y);
    if x_steps != 0 {
        pan_hud_and_focus_right(camera, x_steps);
    }
    if y_steps != 0 {
        pan_hud_and_focus_up(camera, -y_steps);
    }
}

pub fn apply_drawing_space_pan_scroll(camera: &mut Camera, delta_x: f32, delta_y: f32) {
    let x_steps = scroll_step_count(delta_x);
    let y_steps = scroll_step_count(delta_y);
    if x_steps != 0 {
        camera.pan_focus_right(x_steps);
    }
    if y_steps != 0 {
        camera.pan_focus_up(-y_steps);
    }
}

pub fn apply_drawing_space_depth_scroll(camera: &mut Camera, delta_y: f32) {
    let y_steps = scroll_step_count(delta_y);
    if y_steps != 0 {
        camera.pan_focus_depth(y_steps);
    }
}

pub fn apply_drawing_space_scroll(
    camera: &mut Camera,
    mode: DrawingSpaceWheelMode,
    delta_x: f32,
    delta_y: f32,
) {
    match mode {
        DrawingSpaceWheelMode::Pan => apply_drawing_space_pan_scroll(camera, delta_x, delta_y),
        DrawingSpaceWheelMode::Depth => apply_drawing_space_depth_scroll(camera, delta_y),
        // Time is not a camera concern: the consumer steps the playhead
        // against the document's loop window instead.
        DrawingSpaceWheelMode::Time => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_renderer_domain::{CameraRoll, CameraSwing, CellPoint};

    #[test]
    fn canvas_bounds_follow_hud_pan_offset() {
        let at_rest = canvas_bounds_for_viewport(
            INITIAL_PAINT_CANVAS_VIEWPORT,
            &Camera::default(),
            WorldPoint::origin(),
        );
        let panned = canvas_bounds_for_viewport(
            INITIAL_PAINT_CANVAS_VIEWPORT,
            &Camera {
                hud_pan_offset: CellPoint { x: 4, y: -3, z: 0 },
                ..Camera::default()
            },
            WorldPoint::origin(),
        );

        assert_eq!(panned.x0, at_rest.x0 + 4);
        assert_eq!(panned.x1, at_rest.x1 + 4);
        assert_eq!(panned.y0, at_rest.y0 - 3);
        assert_eq!(panned.y1, at_rest.y1 - 3);
    }

    #[test]
    fn canvas_bounds_shift_into_document_space_by_the_move_offset() {
        // The bounds gate move-corrected document positions, so a non-default
        // layer move must shift the bounds by the SAME offset — otherwise the
        // on-screen input region drifts away from the visible drawing-space
        // rect (J 2026-09-10).
        let at_rest = canvas_bounds_for_viewport(
            INITIAL_PAINT_CANVAS_VIEWPORT,
            &Camera::default(),
            WorldPoint::origin(),
        );
        let moved = canvas_bounds_for_viewport(
            INITIAL_PAINT_CANVAS_VIEWPORT,
            &Camera::default(),
            WorldPoint { x: 3, y: -2, z: 5 },
        );

        assert_eq!(moved.x0, at_rest.x0 - 3);
        assert_eq!(moved.x1, at_rest.x1 - 3);
        assert_eq!(moved.y0, at_rest.y0 + 2);
        assert_eq!(moved.y1, at_rest.y1 + 2);
        // A depth move re-anchors the bounds' plane to the layer's document
        // plane, so input stays possible at all.
        assert_eq!(moved.z, at_rest.z - 5);
    }

    #[test]
    fn swing_keeps_viewport_center_world_point_under_viewport_center() {
        let viewport = INITIAL_PAINT_CANVAS_VIEWPORT;
        let before = Camera {
            focus_target: WorldPoint { x: 8, y: -5, z: 2 },
            hud_pan_offset: CellPoint { x: 3, y: -1, z: 0 },
            swing: CameraSwing::PosZ,
            roll: CameraRoll::Deg0,
            ..Camera::default()
        };
        let center = viewport_content_center_in_camera_units(viewport, &before);
        let anchor = remap_camera_units_to_active_plane_world(before, center);
        let mut after = before;

        reorient_camera_around_viewport_center(&mut after, viewport, |camera| camera.swing_right());

        let projected = thaum_renderer_domain::project_world_to_view_plane(after, anchor);
        assert_eq!([projected.u, projected.v], center);
    }

    #[test]
    fn roll_keeps_viewport_center_world_point_under_viewport_center() {
        let viewport = INITIAL_PAINT_CANVAS_VIEWPORT;
        let before = Camera {
            focus_target: WorldPoint { x: -4, y: 7, z: 1 },
            hud_pan_offset: CellPoint { x: -2, y: 5, z: 0 },
            swing: CameraSwing::PosX,
            roll: CameraRoll::Deg90,
            ..Camera::default()
        };
        let center = viewport_content_center_in_camera_units(viewport, &before);
        let anchor = remap_camera_units_to_active_plane_world(before, center);
        let mut after = before;

        reorient_camera_around_viewport_center(&mut after, viewport, |camera| {
            camera.roll = camera.roll.rotate_clockwise()
        });

        let projected = thaum_renderer_domain::project_world_to_view_plane(after, anchor);
        assert_eq!([projected.u, projected.v], center);
    }

    #[test]
    fn hud_scroll_follows_visual_scroll_direction_and_counterpans_3d() {
        let mut camera = Camera::default();

        apply_hud_scroll(&mut camera, -1.0, 1.0);

        assert_eq!(camera.hud_pan_offset.x, -1);
        assert_eq!(camera.hud_pan_offset.y, -1);
        assert_eq!(camera.focus_target.x, 1);
        assert_eq!(camera.focus_target.y, 1);
    }

    #[test]
    fn drawing_space_pan_scroll_follows_visual_scroll_direction() {
        let mut camera = Camera::default();

        apply_drawing_space_pan_scroll(&mut camera, -1.0, 1.0);

        assert_eq!(camera.focus_target.x, -1);
        assert_eq!(camera.focus_target.y, -1);
    }

    #[test]
    fn drawing_space_depth_scroll_moves_forward_on_scroll_up() {
        let mut camera = Camera::default();

        apply_drawing_space_depth_scroll(&mut camera, 1.0);

        assert_eq!(camera.focus_target.z, 1);
    }
}
