//! Tool presets: named two-hand snapshots a user saves once and re-applies
//! with one click or one number key. A preset pairs two half-snapshots
//! (`left` / `right`), each a tool plus only the hand fields the user opted
//! into at save time — so a "brush" preset can carry the tool alone and
//! leave the hand's color/graphic/weight alone, while a "picker" preset can
//! carry its select-opposite toggle with it.
//!
//! The sustainable core is the [`HAND_FIELDS`] registry: one descriptor per
//! capture-able hand field, declared once. Every new hand property becomes
//! one registry line and automatically gains save-toggle support, apply
//! support, and the presets module's toggle rows — no per-tool special
//! cases, nothing hard-coded in the module UI.
//!
//! Lifetime (J 2026-09-24): presets ride the machine-local user session
//! state, never the painting document, so a user's set follows them across
//! every document and survives restarts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tool_state::{HandState, PaintHand, PaintTool, ToolState};
use crate::user_session_state::{
    PersistedChannelMask, PersistedGraphic, PersistedHandState, PersistedPaintColor,
};

/// One capture-able hand field: a stable key, a human label for the save
/// toggles, and get/set over the persisted hand shape (the same shape user
/// session state already trusts). A field's value crosses the registry as
/// JSON so one descriptor fits every value kind without an enum mirroring
/// the persisted types.
pub struct HandFieldDescriptor {
    /// Stable registry key, also the JSON field key inside a preset.
    pub key: &'static str,
    /// Label shown by the presets module's save toggles.
    pub label: &'static str,
    /// Read the field's value out of a persisted hand snapshot.
    pub get: fn(&PersistedHandState) -> Value,
    /// Write a value into a persisted hand snapshot; malformed values are
    /// ignored (the field simply keeps its existing value).
    pub set: fn(&mut PersistedHandState, &Value),
}

fn graphic_get(hand: &PersistedHandState) -> Value {
    serde_json::to_value(&hand.graphic).unwrap_or(Value::Null)
}

fn graphic_set(hand: &mut PersistedHandState, value: &Value) {
    if let Ok(graphic) = serde_json::from_value::<PersistedGraphic>(value.clone()) {
        hand.graphic = graphic;
    }
}

fn color_get(hand: &PersistedHandState) -> Value {
    serde_json::to_value(&hand.color).unwrap_or(Value::Null)
}

fn color_set(hand: &mut PersistedHandState, value: &Value) {
    if let Ok(color) = serde_json::from_value::<PersistedPaintColor>(value.clone()) {
        hand.color = color;
    }
}

/// The full capture field registry, in presets-module toggle order. New hand
/// properties join here — one entry — and every consumer (save toggles,
/// capture, apply) picks them up automatically.
pub const HAND_FIELDS: &[HandFieldDescriptor] = &[
    HandFieldDescriptor {
        key: "graphic",
        label: "graphic",
        get: graphic_get,
        set: graphic_set,
    },
    HandFieldDescriptor {
        key: "color",
        label: "color/material",
        get: color_get,
        set: color_set,
    },
    HandFieldDescriptor {
        key: "weight",
        label: "weight",
        get: |hand| serde_json::to_value(hand.weight_index).unwrap_or(Value::Null),
        set: |hand, value| {
            if let Some(weight) = value.as_i64() {
                hand.weight_index = weight.clamp(0, 3);
            }
        },
    },
    HandFieldDescriptor {
        key: "brush_size",
        label: "brush size",
        get: |hand| serde_json::to_value(hand.brush_size).unwrap_or(Value::Null),
        set: |hand, value| {
            if let Some(size) = value.as_i64() {
                hand.brush_size = (size as i32).clamp(1, 5);
            }
        },
    },
    HandFieldDescriptor {
        key: "fill_diagonal",
        label: "fill diagonal",
        get: |hand| serde_json::to_value(hand.fill_diagonal).unwrap_or(Value::Null),
        set: |hand, value| {
            if let Some(fill) = value.as_bool() {
                hand.fill_diagonal = fill;
            }
        },
    },
    HandFieldDescriptor {
        key: "pick_opposite_hand",
        label: "pick opposite hand",
        get: |hand| serde_json::to_value(hand.pick_opposite_hand).unwrap_or(Value::Null),
        set: |hand, value| {
            if let Some(pick) = value.as_bool() {
                hand.pick_opposite_hand = pick;
            }
        },
    },
    HandFieldDescriptor {
        key: "edit_channels",
        label: "edit channels",
        get: |hand| serde_json::to_value(&hand.edit_channels).unwrap_or(Value::Null),
        set: |hand, value| {
            if let Ok(mask) = serde_json::from_value::<PersistedChannelMask>(value.clone()) {
                hand.edit_channels = mask;
            }
        },
    },
    HandFieldDescriptor {
        key: "select_channels",
        label: "select channels",
        get: |hand| serde_json::to_value(&hand.select_channels).unwrap_or(Value::Null),
        set: |hand, value| {
            if let Ok(mask) = serde_json::from_value::<PersistedChannelMask>(value.clone()) {
                hand.select_channels = mask;
            }
        },
    },
    HandFieldDescriptor {
        key: "target",
        label: "paint target",
        get: |hand| serde_json::to_value(&hand.target).unwrap_or(Value::Null),
        set: |hand, value| {
            if let Some(target) = value.as_str() {
                hand.target = target.to_string();
            }
        },
    },
];

/// Resolve one registry descriptor by key.
pub fn hand_field(key: &str) -> Option<&'static HandFieldDescriptor> {
    HAND_FIELDS.iter().find(|field| field.key == key)
}

/// One hand's half of a preset: the tool it equips plus the captured field
/// values by registry key. Tool is always included (J 2026-09-24: "yes it is
/// for tools"); fields carry only what the user opted into at save time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPresetHand {
    /// Stable tool id ("brush"), resolved through the painter-tools registry.
    pub tool: String,
    /// Captured field values by registry key. Absent keys leave the hand's
    /// current value alone when the preset applies.
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
}

impl ToolPresetHand {
    /// Capture one hand's snapshot: its tool plus only the field keys in
    /// `capture_keys`.
    pub fn capture(tool: PaintTool, hand: &HandState, capture_keys: &std::collections::BTreeSet<String>) -> Self {
        let persisted = PersistedHandState::from_runtime(hand);
        let fields = HAND_FIELDS
            .iter()
            .filter(|field| capture_keys.contains(field.key))
            .map(|field| (field.key.to_string(), (field.get)(&persisted)))
            .collect();
        Self {
            tool: tool.id().to_string(),
            fields,
        }
    }

    /// A tool-only half: equips the tool, overrides nothing.
    pub fn tool_only(tool: PaintTool) -> Self {
        Self {
            tool: tool.id().to_string(),
            fields: BTreeMap::new(),
        }
    }

    /// Apply this half onto one runtime hand: equip the tool, then overlay
    /// only the captured fields (an uncaptured field keeps the hand's
    /// current value). Field values round-trip through a persisted snapshot
    /// so apply and save trust the exact same shape.
    pub fn apply_to_runtime(&self, tool_state: &mut ToolState, hand: PaintHand) {
        if let Some(tool) = PaintTool::from_id(&self.tool) {
            tool_state.set_tool_for_hand(hand, tool);
        }
        let mut updated = tool_state.hand_state(hand);
        let mut scratch = PersistedHandState::from_runtime(&updated);
        for (key, value) in &self.fields {
            if let Some(field) = hand_field(key) {
                (field.set)(&mut scratch, value);
            }
        }
        scratch.apply_to_runtime(&mut updated);
        match hand {
            PaintHand::Left => tool_state.left_hand = updated,
            PaintHand::Right => tool_state.right_hand = updated,
        }
    }
}

/// A named two-hand tool preset. Left click applies the left half onto the
/// left hand, right click the right half, middle click both, and a number
/// key applies both halves (J 2026-09-24).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPreset {
    /// User-editable display name (renamed through the presets module).
    pub name: String,
    pub left: ToolPresetHand,
    pub right: ToolPresetHand,
}

impl ToolPreset {
    /// Capture both live hands as one preset with `capture_keys` deciding
    /// which fields ride along.
    pub fn capture(
        name: impl Into<String>,
        tool_state: &ToolState,
        capture_keys: &std::collections::BTreeSet<String>,
    ) -> Self {
        Self {
            name: name.into(),
            left: ToolPresetHand::capture(tool_state.left_tool, &tool_state.left_hand, capture_keys),
            right: ToolPresetHand::capture(
                tool_state.right_tool,
                &tool_state.right_hand,
                capture_keys,
            ),
        }
    }

    /// Apply both halves: left half onto the left hand, right half onto the
    /// right hand. The number-key path and middle click both land here.
    pub fn apply_to_runtime(&self, tool_state: &mut ToolState) {
        self.left.apply_to_runtime(tool_state, PaintHand::Left);
        self.right.apply_to_runtime(tool_state, PaintHand::Right);
    }
}

/// The starter presets shipped as presets 1-2 so the module is useful before
/// anything is saved (J 2026-09-24: "yes give it a try since you are making
/// the format anyway"). They are J's two real painting modes:
///
/// 1. "select + move" — left lasso, right move.
/// 2. "paint + pick" — left brush, right picker with select-opposite.
///
/// The brush halves carry the tool only — no color/graphic/weight override
/// (J: the brush in those setups should not stomp the hand's current
/// appearance).
pub fn starter_presets() -> Vec<ToolPreset> {
    let mut pick_opposite = BTreeMap::new();
    pick_opposite.insert(
        "pick_opposite_hand".to_string(),
        serde_json::json!(true),
    );
    vec![
        ToolPreset {
            name: "select + move".to_string(),
            left: ToolPresetHand::tool_only(PaintTool::Lasso),
            right: ToolPresetHand::tool_only(PaintTool::Move),
        },
        ToolPreset {
            name: "paint + pick".to_string(),
            left: ToolPresetHand::tool_only(PaintTool::Brush),
            right: ToolPresetHand {
                tool: PaintTool::Picker.id().to_string(),
                fields: pick_opposite,
            },
        },
    ]
}

/// How many presets the number keys reach (top-row digits 1-9, J
/// 2026-09-24; the numpad stays on the camera keys).
pub const NUMBER_KEY_SLOTS: usize = 9;

/// The preset a number key applies: digit 1 = the first preset, digit 9 =
/// the ninth. Presets beyond the list stay click-only.
pub fn preset_for_number_slot(presets: &[ToolPreset], slot: usize) -> Option<&ToolPreset> {
    if slot == 0 || slot > NUMBER_KEY_SLOTS {
        return None;
    }
    presets.get(slot - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_state::PaintTarget;
    use thaum_renderer_domain::CellGraphic;

    fn keys(all: bool) -> std::collections::BTreeSet<String> {
        HAND_FIELDS
            .iter()
            .filter(|field| all || field.key == "brush_size")
            .map(|field| field.key.to_string())
            .collect()
    }

    #[test]
    fn hand_field_registry_keys_are_unique() {
        let mut collected: Vec<&str> = HAND_FIELDS.iter().map(|field| field.key).collect();
        collected.sort_unstable();
        let unique = collected.clone();
        collected.dedup();
        assert_eq!(collected, unique);
    }

    #[test]
    fn capture_with_every_field_toggled_snapshots_the_full_hand() {
        let mut hand = HandState::default();
        hand.graphic = CellGraphic::Glyph('&');
        hand.weight_index = 3;
        hand.brush_size = 4;
        hand.target = PaintTarget::Selection;
        let captured = ToolPresetHand::capture(PaintTool::Brush, &hand, &keys(true));
        assert_eq!(captured.tool, "brush");
        assert_eq!(captured.fields.len(), HAND_FIELDS.len());
    }

    #[test]
    fn capture_with_one_field_toggled_carries_only_that_field() {
        let mut hand = HandState::default();
        hand.weight_index = 1;
        hand.brush_size = 4;
        let captured = ToolPresetHand::capture(PaintTool::Fill, &hand, &keys(false));
        assert_eq!(captured.tool, "fill");
        assert_eq!(captured.fields.keys().collect::<Vec<_>>(), ["brush_size"]);
        assert_eq!(captured.fields["brush_size"], serde_json::json!(4));
    }

    #[test]
    fn apply_overrides_only_captured_fields() {
        let mut hand = HandState::default();
        hand.brush_size = 4;
        let mut tool_state = ToolState::default();
        // The left hand starts at glyph '#' weight 2 (defaults); the preset
        // captures only the brush size, so applying it must not stomp the
        // hand's appearance.
        let captured = ToolPresetHand::capture(PaintTool::Fill, &hand, &keys(false));
        captured.apply_to_runtime(&mut tool_state, PaintHand::Left);
        let left = tool_state.hand_state(PaintHand::Left);
        assert_eq!(left.graphic, CellGraphic::Glyph('#'));
        assert_eq!(left.weight_index, 2);
        assert_eq!(left.brush_size, 4);
        assert_eq!(tool_state.left_tool, PaintTool::Fill);
    }

    #[test]
    fn apply_on_the_right_hand_leaves_the_left_hand_alone() {
        let captured = ToolPresetHand::tool_only(PaintTool::Lasso);
        captured.apply_to_runtime(&mut ToolState::default(), PaintHand::Right);
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Brush);
        captured.apply_to_runtime(&mut tool_state, PaintHand::Right);
        assert_eq!(tool_state.left_tool, PaintTool::Brush);
        assert_eq!(tool_state.right_tool, PaintTool::Lasso);
    }

    #[test]
    fn both_halves_apply_to_their_own_hands() {
        let preset = ToolPreset {
            name: "pair".to_string(),
            left: ToolPresetHand::tool_only(PaintTool::Lasso),
            right: ToolPresetHand::tool_only(PaintTool::Move),
        };
        let mut tool_state = ToolState::default();
        preset.apply_to_runtime(&mut tool_state);
        assert_eq!(tool_state.left_tool, PaintTool::Lasso);
        assert_eq!(tool_state.right_tool, PaintTool::Move);
    }

    #[test]
    fn preset_survives_a_serde_round_trip() {
        let mut pick_opposite = BTreeMap::new();
        pick_opposite.insert("pick_opposite_hand".to_string(), serde_json::json!(true));
        let preset = ToolPreset {
            name: "paint + pick".to_string(),
            left: ToolPresetHand::tool_only(PaintTool::Brush),
            right: ToolPresetHand {
                tool: "picker".to_string(),
                fields: pick_opposite,
            },
        };
        let text = serde_json::to_string(&preset).unwrap();
        let restored: ToolPreset = serde_json::from_str(&text).unwrap();
        assert_eq!(restored, preset);
    }

    #[test]
    fn starter_presets_match_the_two_declared_modes() {
        let starters = starter_presets();
        assert_eq!(starters.len(), 2);
        // Mode 1: left lasso, right move.
        assert_eq!(starters[0].left.tool, "lasso");
        assert_eq!(starters[0].right.tool, "move");
        assert!(starters[0].left.fields.is_empty());
        // Mode 2: left brush (tool only — no color/graphic/weight override),
        // right picker with select-opposite.
        assert_eq!(starters[1].left.tool, "brush");
        assert!(starters[1].left.fields.is_empty());
        assert_eq!(starters[1].right.tool, "picker");
        assert_eq!(
            starters[1].right.fields["pick_opposite_hand"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn number_slots_resolve_one_based_and_stop_at_nine() {
        let starters = starter_presets();
        assert_eq!(preset_for_number_slot(&starters, 1).map(|p| p.name.as_str()), Some("select + move"));
        assert_eq!(preset_for_number_slot(&starters, 2).map(|p| p.name.as_str()), Some("paint + pick"));
        assert!(preset_for_number_slot(&starters, 3).is_none());
        assert!(preset_for_number_slot(&starters, 0).is_none());
        assert!(preset_for_number_slot(&starters, 10).is_none());
    }

    #[test]
    fn apply_lands_the_picker_select_opposite_toggle_on_the_right_hand() {
        let starters = starter_presets();
        let mut tool_state = ToolState::default();
        starters[1].apply_to_runtime(&mut tool_state);
        let right = tool_state.hand_state(PaintHand::Right);
        assert!(right.pick_opposite_hand);
        assert_eq!(tool_state.right_tool, PaintTool::Picker);
        // The left brush keeps its default appearance: the starter never
        // overrides color/graphic/weight.
        let left = tool_state.hand_state(PaintHand::Left);
        assert_eq!(left.graphic, CellGraphic::Glyph('#'));
        assert_eq!(left.color, crate::paint_color::PaintColor::default());
        assert_eq!(left.weight_index, 2);
    }
}
