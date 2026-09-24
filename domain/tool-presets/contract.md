# domain/tool-presets

## purpose
tool presets: named two-hand snapshots of tool + opted-in hand properties that a user saves once and re-applies with one click or one number key.

## owns
- the `HAND_FIELDS` capture-field registry: one descriptor per capture-able hand field (key, label, get/set over the persisted hand shape)
- the `ToolPreset` / `ToolPresetHand` serde shape: left/right halves, each tool + captured field map
- capture (`ToolPreset::capture`) and apply (`ToolPreset::apply_to_runtime`) semantics
- starter presets and the number-key slot resolution (top-row digits 1-9)

## does not own
- `HandState` / `ToolState` truth (`painter-session/tool-state`)
- the persisted hand shape (`user-state/user-session-state` — the registry's get/set read/write `PersistedHandState` but user-session-state owns it)
- the presets module UI (`modules/individuals/tool-presets` — save toggles, rename, delete)
- painter tool registration (`painter-tools`)
- preset persistence location (`user-state` rides the user session state file)

## decisions
- 2026-09-24 J: a preset is for tools — the tool is always part of a preset; fields carry only what the user opted into at save time.
- 2026-09-24 J: field capture is registry-driven, not hard-coded — every new hand property becomes one `HAND_FIELDS` entry and automatically gains save toggles, capture, and apply.
- 2026-09-24 J: presets are user bound, not file bound — they ride the machine-local user session state, never the painting document.
- 2026-09-24 J: number keys are the top-row digits 1-9 applying to both hands; the numpad stays on the camera keys.
- 2026-09-24 J: starter presets 1-2 are J's two real modes (lasso+move, brush+picker-with-select-opposite); the brush halves carry the tool only so they never override the hand's color/graphic/weight.

## notes
- field values cross the registry as JSON so one descriptor fits every value kind without mirroring the persisted types; malformed values are ignored on apply (the field keeps its current value).
- apply overlays only captured fields onto a persisted snapshot of the live hand, so uncaptured fields are preserved exactly.
