# domain/modules/individuals/tool-presets

## purpose
toolbox-sibling presets panel: lists the user's saved two-hand tool presets and applies them per hand; owns the SAVE row, the registry-generated capture toggles, inline rename, and delete.

## owns
- the presets module's draw, hit-testing, hotspots, and pointer interactions
- the SAVE row's capture-toggle set (which `HAND_FIELDS` registry fields ride into a new preset)
- inline rename (double left click on a preset row) and delete (double right click) interactions

## does not own
- the preset data itself — `domain/tool-presets` owns the shape/capture/apply; the module edits a shared `Rc<RefCell<Vec<ToolPreset>>>` handle
- preset persistence — `user-state/user-session-state` rides the user session file
- number-key application — orchestration-owned key dispatch applies both halves; the module only shows the slot digit
- tool state truth (`painter-session/tool-state`)

## interactions
- preset rows at the top (above SAVE): left click -> left half onto left hand, right click -> right half onto right hand, middle click -> both; double left click renames inline (shared text-entry + click-timing seams), double right click deletes (J 2026-09-24)
- SAVE row (left click): captures both live hands, filtered by the toggles, auto-named `preset N`
- `save options` line below SAVE: one collapsed line by default; click expands it into one toggle row per `HAND_FIELDS` registry entry, click again contracts (J 2026-09-24)
- toggle rows (while expanded): one per `HAND_FIELDS` registry entry, click to include/exclude a field — new hand properties appear here for free (J 2026-09-24: don't hard-code this)

## notes
- preset rows read top-down above SAVE so the 1-9 slot digits sit at the top of the panel; SAVE anchors below them, then the collapsible save-options line
- row indicators mirror the toolbox: `L`/`R`/`◆` when the live hands carry the preset's tools
