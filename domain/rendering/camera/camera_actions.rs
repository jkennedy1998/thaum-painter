//! Painter camera actions: one dispatch seam for every registry camera
//! action (pan, swing, roll, depth focus, zoom). The entrypoint resolves
//! action names through the binding map and calls this seam; how each
//! action moves the camera — including the hover-dependent 3D/HUD pan
//! split — lives here, not in the event loop.

use thaum_renderer_domain::{Camera, ModuleRect};

use crate::camera_viewport::{
    pan_hud_and_focus_right, pan_hud_and_focus_up, reorient_camera_around_viewport_center,
};

/// Applies a held pan action. Continuous pan (WASD) resolves through the
/// same seam as press dispatch so a remap moves the behavior with it.
/// Hovering the drawing surface routes to 3D pan (the file's contents,
/// under a screen-fixed frame); anywhere else routes to 2D pan (the HUD
/// layer itself, so off-screen panels can be reached).
pub fn apply_painter_pan_action(camera: &mut Camera, action: &str, hovering_canvas: bool) -> bool {
    // Inverted from the camera's own right/up so the content visually moves
    // the way the key points, not the way the camera's aim point moves.
    match action {
        "painter_pan_left" => {
            if hovering_canvas {
                camera.pan_focus_right(1);
            } else {
                pan_hud_and_focus_right(camera, 1);
            }
        }
        "painter_pan_right" => {
            if hovering_canvas {
                camera.pan_focus_right(-1);
            } else {
                pan_hud_and_focus_right(camera, -1);
            }
        }
        "painter_pan_up" => {
            if hovering_canvas {
                camera.pan_focus_up(-1);
            } else {
                pan_hud_and_focus_up(camera, -1);
            }
        }
        "painter_pan_down" => {
            if hovering_canvas {
                camera.pan_focus_up(1);
            } else {
                pan_hud_and_focus_up(camera, 1);
            }
        }
        _ => return false,
    }
    true
}

/// Applies one press-dispatch camera action. Returns whether the action was
/// consumed; unknown names are left for the caller's remaining dispatch.
pub fn apply_painter_camera_action(
    camera: &mut Camera,
    action: &str,
    viewport: ModuleRect,
    hovering_canvas: bool,
) -> bool {
    if apply_painter_pan_action(camera, action, hovering_canvas) {
        return true;
    }
    match action {
        "painter_swing_left" => {
            reorient_camera_around_viewport_center(camera, viewport, |camera| camera.swing_left())
        }
        "painter_swing_right" => {
            reorient_camera_around_viewport_center(camera, viewport, |camera| camera.swing_right())
        }
        "painter_swing_up" => {
            reorient_camera_around_viewport_center(camera, viewport, |camera| camera.swing_up())
        }
        "painter_swing_down" => {
            reorient_camera_around_viewport_center(camera, viewport, |camera| camera.swing_down())
        }
        "painter_roll_counter_clockwise" => reorient_camera_around_viewport_center(
            camera,
            viewport,
            |camera| camera.roll = camera.roll.rotate_counter_clockwise(),
        ),
        "painter_roll_clockwise" => reorient_camera_around_viewport_center(camera, viewport, |camera| {
            camera.roll = camera.roll.rotate_clockwise()
        }),
        "painter_focus_depth_toward" => camera.pan_focus_depth(-1),
        "painter_focus_depth_away" => camera.pan_focus_depth(1),
        "painter_zoom_out" => camera.zoom_out(),
        "painter_zoom_in" => camera.zoom_in(),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_viewport::INITIAL_PAINT_CANVAS_VIEWPORT;
    use thaum_renderer_domain::WorldPoint;

    #[test]
    fn pan_actions_split_between_hud_and_canvas_routes() {
        let mut hud_pan = Camera::default();
        assert!(apply_painter_pan_action(&mut hud_pan, "painter_pan_left", false));
        assert!(hud_pan.hud_pan_offset.x > 0);
        assert_ne!(hud_pan.focus_target, WorldPoint::origin());

        let mut canvas_pan = Camera::default();
        assert!(apply_painter_pan_action(
            &mut canvas_pan,
            "painter_pan_left",
            true
        ));
        assert_eq!(canvas_pan.hud_pan_offset.x, 0);
        assert_ne!(canvas_pan.focus_target, WorldPoint::origin());
        assert_ne!(hud_pan.focus_target, canvas_pan.focus_target);
    }

    #[test]
    fn press_actions_cover_swing_roll_depth_and_zoom() {
        for action in [
            "painter_swing_left",
            "painter_swing_right",
            "painter_swing_up",
            "painter_swing_down",
            "painter_roll_counter_clockwise",
            "painter_roll_clockwise",
            "painter_focus_depth_toward",
            "painter_focus_depth_away",
            "painter_zoom_out",
            "painter_zoom_in",
        ] {
            let mut camera = Camera::default();
            assert!(
                apply_painter_camera_action(
                    &mut camera,
                    action,
                    INITIAL_PAINT_CANVAS_VIEWPORT,
                    false
                ),
                "{action} must be a camera action"
            );
        }
    }

    #[test]
    fn unknown_actions_are_left_for_the_caller() {
        let mut camera = Camera::default();
        assert!(!apply_painter_pan_action(&mut camera, "painter_play_pause", false));
        assert!(!apply_painter_camera_action(
            &mut camera,
            "painter_play_pause",
            INITIAL_PAINT_CANVAS_VIEWPORT,
            false
        ));
    }
}
