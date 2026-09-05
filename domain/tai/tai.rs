//! Painter-side tool-assisted input (TAI) seam.
//!
//! Consumes the renderer's TAI runner (`thaum_renderer_domain::controls::tai`)
//! and owns the painter-specific half: painter action bindings, the growable
//! `individuals/` script list, and `individuals/registry.json`. Adding a TAI is
//! copy-template -> fill script -> one registry line; this file only changes if
//! a script needs a new painter binding declared.
//!
//! The `painter_bindings()` map is the painter's declared hotkey map. It lives
//! here first as the test-harness source of truth; when the entrypoint adopts
//! it for real input handling, this becomes the painter's one binding surface.

use std::path::Path;

use thaum_renderer_domain::controls::tai::TaiScript;
use thaum_renderer_domain::controls::{ActionBindingMap, ActionName, RawInput};

/// How many breaths a painter TAI run spans.
pub const TAI_TOTAL_BREATHS: u64 = 64;

/// The painter's declared named actions and their bound raw inputs.
/// This is the one hotkey/binding declaration: the TAI harness asserts
/// against it, the entrypoint dispatches live input through it, and the
/// controls panel lists it. User remaps layer on top via the renderer's
/// controls-profile seam (`effective_bindings`).
pub fn painter_bindings() -> ActionBindingMap {
    let mut map = ActionBindingMap::new();
    // Tools (old-system names kept; live mapping is pencil -> Brush,
    // bucket -> Fill).
    map.bind(
        ActionName::new("painter_select_pencil"),
        RawInput::Key("P".to_string()),
    );
    map.bind(
        ActionName::new("painter_select_bucket"),
        RawInput::Key("B".to_string()),
    );
    map.bind(
        ActionName::new("painter_select_lasso"),
        RawInput::Key("L".to_string()),
    );
    map.bind(
        ActionName::new("painter_select_stamp"),
        RawInput::Key("V".to_string()),
    );
    // Drawing-space pan (context decides 3D focus pan vs HUD pan).
    map.bind(ActionName::new("painter_pan_left"), RawInput::Key("A".to_string()));
    map.bind(ActionName::new("painter_pan_right"), RawInput::Key("D".to_string()));
    map.bind(ActionName::new("painter_pan_up"), RawInput::Key("W".to_string()));
    map.bind(ActionName::new("painter_pan_down"), RawInput::Key("S".to_string()));
    // Playback transport (also on the layers-panel loop-bar row).
    map.bind(
        ActionName::new("painter_play_pause"),
        RawInput::Key("SPACE".to_string()),
    );
    // Camera swing / roll / focus-depth.
    map.bind(
        ActionName::new("painter_swing_left"),
        RawInput::Key("NUMPAD4".to_string()),
    );
    map.bind(
        ActionName::new("painter_swing_right"),
        RawInput::Key("NUMPAD6".to_string()),
    );
    map.bind(
        ActionName::new("painter_swing_up"),
        RawInput::Key("NUMPAD8".to_string()),
    );
    map.bind(
        ActionName::new("painter_swing_down"),
        RawInput::Key("NUMPAD2".to_string()),
    );
    map.bind(
        ActionName::new("painter_roll_counter_clockwise"),
        RawInput::Key("NUMPAD7".to_string()),
    );
    map.bind(
        ActionName::new("painter_roll_clockwise"),
        RawInput::Key("NUMPAD9".to_string()),
    );
    map.bind(
        ActionName::new("painter_focus_depth_toward"),
        RawInput::Key("NUMPAD1".to_string()),
    );
    map.bind(
        ActionName::new("painter_focus_depth_away"),
        RawInput::Key("NUMPAD3".to_string()),
    );
    // Zoom: keys plus the TAI wheel binding.
    map.bind(ActionName::new("painter_zoom_out"), RawInput::Key("-".to_string()));
    map.bind(
        ActionName::new("painter_zoom_out"),
        RawInput::Key("NUMPAD_SUB".to_string()),
    );
    map.bind(ActionName::new("painter_zoom_in"), RawInput::Key("=".to_string()));
    map.bind(
        ActionName::new("painter_zoom_in"),
        RawInput::Key("NUMPAD_ADD".to_string()),
    );
    map.bind(ActionName::new("painter_zoom_in"), RawInput::MouseWheelUp);
    // Selection modes.
    map.bind(
        ActionName::new("painter_selection_mode_replace"),
        RawInput::Key("1".to_string()),
    );
    map.bind(
        ActionName::new("painter_selection_mode_additive"),
        RawInput::Key("2".to_string()),
    );
    map.bind(
        ActionName::new("painter_selection_mode_subtract"),
        RawInput::Key("3".to_string()),
    );
    map.bind(
        ActionName::new("painter_selection_mode_intersect"),
        RawInput::Key("4".to_string()),
    );
    // Selection plane operations.
    map.bind(
        ActionName::new("painter_selection_clear"),
        RawInput::Key("C".to_string()),
    );
    map.bind(
        ActionName::new("painter_selection_invert"),
        RawInput::Key("I".to_string()),
    );
    map.bind(
        ActionName::new("painter_selection_all"),
        RawInput::Key("X".to_string()),
    );
    // Shared-document history.
    map.bind(ActionName::new("painter_undo"), RawInput::Key("Z".to_string()));
    map.bind(ActionName::new("painter_redo"), RawInput::Key("Y".to_string()));
    // Clipboard: copy the selection as a 3D world copy. Pasting is the
    // stamp tool (V above); a THAUM3D OS-clipboard payload imports into the
    // own buffer when stamp is equipped.
    map.bind(
        ActionName::new("painter_clipboard_copy"),
        RawInput::Key("C".to_string()),
    );
    map
}

fn individuals_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tai")
}

/// Load every script listed in `individuals/registry.json`.
/// Registry-driven so a new registry line is automatically picked up.
pub fn load_registered_scripts() -> Result<Vec<(String, TaiScript)>, String> {
    let registry_path = individuals_root().join("individuals/registry.json");
    let registry_raw = std::fs::read_to_string(&registry_path)
        .map_err(|e| format!("tai_registry_unreadable {}: {e}", registry_path.display()))?;
    let registry: serde_json::Value =
        serde_json::from_str(&registry_raw).map_err(|e| format!("tai_registry_invalid: {e}"))?;
    let entries = registry
        .as_object()
        .ok_or_else(|| "tai_registry_not_an_object".to_string())?;

    let mut loaded = Vec::new();
    for (id, entry) in entries {
        let script_rel = entry
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| format!("tai_registry_entry_missing_path:{id}"))?;
        let script_path = individuals_root().join(script_rel);
        let raw = std::fs::read_to_string(&script_path)
            .map_err(|e| format!("tai_script_unreadable {script_rel}: {e}"))?;
        let script = TaiScript::parse(&raw).map_err(|e| format!("tai {id}: {e}"))?;
        loaded.push((id.clone(), script));
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_not_empty() {
        let scripts = load_registered_scripts().expect("registry loads");
        assert!(!scripts.is_empty(), "painter tai registry is empty");
    }

    #[test]
    fn every_registered_tai_parses_and_passes() {
        let bindings = painter_bindings();
        for (id, script) in load_registered_scripts().expect("registry loads") {
            let report = script.run(&bindings, TAI_TOTAL_BREATHS);
            assert!(
                report.passed(),
                "tai {id} misses: {:?}",
                report.expectation_misses
            );
        }
    }

    #[test]
    fn registered_tool_hotkeys_match_declared_bindings() {
        let bindings = painter_bindings();
        for descriptor in crate::painter_tools::all() {
            let (Some(action), Some(hotkey)) = (descriptor.select_action, descriptor.hotkey)
            else {
                continue;
            };
            assert_eq!(
                bindings.bindings_for(&ActionName::new(action)),
                &[RawInput::Key(hotkey.to_string())],
                "tool {} expects {hotkey} bound to {action}",
                descriptor.id
            );
        }
    }

    #[test]
    fn smoke_tai_fires_exactly_the_expected_actions() {
        let bindings = painter_bindings();
        let scripts = load_registered_scripts().expect("registry loads");
        let (_, script) = scripts
            .iter()
            .find(|(id, _)| id == "01")
            .expect("smoke tai registered");
        let report = script.run(&bindings, TAI_TOTAL_BREATHS);
        assert_eq!(
            report.fired,
            vec![
                (1, "painter_select_pencil".to_string()),
                (3, "painter_select_bucket".to_string()),
                (5, "painter_zoom_in".to_string())
            ]
        );
    }
}
