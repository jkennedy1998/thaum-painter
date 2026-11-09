# project plan — vector tool + lasso/vector stroke & fill

## pre-implementation-note
J wants a new vector/handle drawing tool (press-only = hard-corner point, press+drag = a handled point with mirrored forward/backward handles) and a new stroke/fill authoring pair shared across `lasso` and the new `vector` tool, each sourced from a hand's selected graphic/color/weight or `none`. Lasso stays a freehand-bound 2D tool; vector is a separate registered tool, designed 2D-first but with 3D handle depth in mind. Shape tools (2D and 3D) are explicitly future work that will reuse the same stroke/fill plumbing — out of bounds here. Design truths are recorded in `domain/painter-tools/truth.md` (2026-11-09 entries). No formal jobo project id — this plan is the sole canonical checklist, colocated in this repo's `plans/`.

## current-state
- `lasso` (`domain/painter-tools/individuals/lasso/`) is `DragBehavior::ReleaseBound`: press-drag-release records a path, release fills the enclosed cells. Its only property row today is `fill_3d` (whether the fill punches through depth).
- No vector/handle tool exists. `PaintTool` (`painter-session/tool-state/tool_state.rs`) has 8 variants and no construction-session concept; `DragBehavior` (`painter-tools/shared/stroke_rules.rs`) has `PerPosition` / `ReleaseBound` / `TypingSession` / `ClickOnly` / `MoveSelection` — nothing models "open across many presses until Enter/Escape/tool-switch commits."
- `canvas-pointer` already owns three in-progress-accumulation patterns worth comparing against: the lasso bound (one single drag-recorded path, `LassoStroke`), the typing session (multi-press, explicit Enter/Escape exit, `TypingSessionBegin`), and `painter-session/selection/selection_stroke.rs`'s `SelectionStroke` — a multi-click/multi-drag `BTreeSet<CellPoint>` accumulator with its own vivid-flash preview. Resolved (2026-11-09): vector gets its own construction type, not a shared one with `SelectionStroke` — selection edits a bitmap-like overlay, vector authors ordered point/handle data for drawn content, and forcing one struct across both would be the "messy" sharing J doesn't want. The *pattern* (incremental multi-press accumulation + a vivid-flash preview) is the reusable precedent; the concrete type is not.
- Lasso's path is always treated as implicitly closed for its fill and has no stroke-only (open) end state. Resolved (2026-11-09): a vector shape closes the same way — connecting its own first and last point within one stroke/subpath. Unlike lasso, one construction session can hold **more than one shape at once**: a user can start a new shape while an earlier one in the same session is still open, and every shape/subpath in the session commits together as one action. Each committed shape is one continuous primary line plus any additional parallel lines (subpaths) authored alongside it in the same construction — this is a multi-subpath compound-shape model, not a single flat point list.
- The hand-authoring seam already exists end-to-end: `tool_state::resolved_painted_cell(existing, hand)` → `brush::write_cell` (blank-resolved cells remove instead of insert) is exactly what lasso's `apply_lasso_for_hand` already uses — stroke/fill's `Left`/`Right` resolution is a direct reuse, not new logic.
- Auditing every `PaintTool::Lasso` call site found the tool-registration ripple is bigger than first scoped: `domain/modules/individuals/toolbox/` hardcodes its own `ToolDef` list (doesn't read the registry generically), `domain/tai/` holds the actual default keybinding (separate from the registry descriptor's informational `hotkey` field), and `orchestration/build-commands/` drift-tests a declared-actions list and a controls/help row list — all three now have their own plan entries (folded into phase-4).
- All five registered letter hotkeys are already taken (`P` brush, `B` fill, `L` lasso, `V` stamp, `M` move), as are several more used by pan/selection/undo/zoom actions (`A`/`D`/`W`/`S`, `C`, `I`, `X`, `Z`, `Y`). J said the exact key doesn't matter and can be revisited later — this plan picks `H` (unused, loosely "handle").
- Committed vector data is rasterize-and-discard for this build (resolved 2026-11-09): the final commit writes only authored cells, no vector path survives. A future "smart vector layer" (a layer that *is* one or more re-editable authored vector shapes) is an explicitly separate, later project — this build's construction-phase architecture (`VectorPoint`, multi-subpath construction) should stay clean enough to feed that later without being over-built for it now.
- Fill math (resolved 2026-11-09, sharpened 2026-11-09): overlapping subpaths/shapes that cover the same cells **subtract from each other** (even-odd fill rule) rather than overwrite — this is what lets a compound shape punch holes in itself (donuts, letterform counters), and it means 2D fill needs a real even-odd interior resolver over every subpath in the session together, not a single-polygon fill reused as-is. Lasso's `lasso_points`/`lasso_points_through_depth` point-in-polygon math may still be a usable primitive inside that resolver (per-subpath containment test), but the even-odd *composition* across subpaths is new code, not a drop-in reuse. 3D vector fill is **not settled** — a simple punch-through-all-depth-layers (mirroring `fill_3d`'s existing convention) is the working fallback, but J wants the real 3D fill UX "nicely thought through" before it's locked in. This plan does not block on that: 2D fill can land on schedule, 3D vector fill is called out below as its own design pass, not a silent gap.
- Selection/edit model (resolved 2026-11-09): exactly one point is "selected" at a time across the whole session (any subpath, not just the active one); only the selected point renders its handles, which both keeps the preview legible and avoids handle/point cell collisions. A plain click on a point selects it; a press+drag moves it. Pressing empty space branches off the selected point's connection count: one connection (open tail) continues that subpath, two connections (fully interior) starts a new subpath rooted there. Closing is a restricted case of pressing a point: pressing the active subpath's own first point while its open tail is selected closes it, modeled on Adobe's pen-tool convention. Precedence on any press: point > handle > create-new-point/close.
- Double-click gives two more authoring actions without a modifier key: on empty space it starts a brand-new independent subpath; on an existing point it toggles that point's handles between zero magnitude (hard corner) and magnitude 1 (grabbable) — a click-only way to add handles to an already-placed point. The full target chart is below; two cells in it (handle-expansion direction, the point-delete gesture) are still open, by design — J wants the chart finished first.
- A single authored point is a valid committed result (stroke renders it once stroke-size exists; fill never applies — no enclosed area). Zero points is not expected to arise directly; the working idea is a point-delete interaction during construction, and deleting every point back to zero becomes the session's only "exit," since the session still only ever commits (possibly nothing) — no separate cancel/discard path is added.
- Hand content (graphic/color/weight) stays live and editable through an open construction session — swapping a hand's selection mid-shape is intentional, not a bug to guard against, so multi-part authoring is easy. `None`/`None` on both hands making the tool do nothing on release is accepted as-is; no forced non-None default was requested.
- Per-hand persisted tool properties (`fill_3d`) round-trip through three places today: `tool-state` (`set_fill_3d_for_hand`), `user-session-state` (session persistence), and `tool-presets` (preset get/set). The new stroke/fill source properties need the same three-way wiring.
- Property-row UI (`modules/individuals/hand-settings/`) currently has two row shapes that could fit a 3-way "Left / Right / None" source: a boolean `Matrix` row (used for `fill_3d`, one bool per hand) or a `ListSwitchRow` (single cycling value, host owns the option list). Neither is an exact precedent for "which hand, or neither" — phase-1 picks one before the hand-settings phase implements it.

## why this plan has an open question going in
J left the property-row shape to "whatever is most standard and clean" — confirmed no exact precedent exists (`fill_3d`'s boolean `Matrix` row doesn't fit a 3-way Left/Right/None). Phase-1 settles the concrete row shape before the hand-settings phase implements it.

Depth is not an open question: focus-depth scroll-while-dragging already exists and the Move tool already folds a mid-drag depth scroll into its accumulated offset (`move_drag_follows_a_mid_drag_focus_depth_scroll_to_the_new_depth`, `move_release_folds_a_depth_scroll_that_followed_the_last_drag_frame` in `canvas_pointer.rs`). Vector construction/move reuses that exact pattern — reads live focus depth at point-spawn and at point/handle-move time, never captures depth once at press-start.

## construction-phase interaction chart (2026-11-09)
Requested by J before the point-delete gesture gets designed. Hit-testing is cell-exact throughout — a press target is whatever point/handle/empty cell the press lands on. Only the currently *selected* point renders its handles; every other point is a plain vertex marker, so a handle is only ever a distinct press target when it belongs to the selected point and has nonzero magnitude.

| press target | gesture | precondition | action |
|---|---|---|---|
| no point yet in this session | single click or press+drag | session just started | place the first point of the first subpath (hard corner on click, handled+mirrored on press+drag) |
| existing point | single click | — | select this point (deselects whatever was selected; its handles become visible) |
| existing point | press+drag | — | rigid-move this point — translates both its handles by the same offset, live focus depth included |
| existing point == active subpath's own first point | single click | the currently selected point is that subpath's own open tail (exactly one connection) | **close the subpath** instead of plain select — points win over this case only when the preconditions line up; otherwise it's an ordinary select |
| existing point's handle (nonzero magnitude, distinct cell from its point) | single click | that point is the one currently selected (handles only render/press when selected) | select the handle for a follow-up drag (handles win over empty-cell create/close) |
| existing point's handle | press+drag | same as above | move this one handle only — asymmetric, sibling handle unaffected |
| existing point | double-click | point currently has nonzero-magnitude handles | collapse both handles to zero magnitude (back to a hard corner) |
| existing point | double-click | point currently has zero-magnitude handles | expand both handles to magnitude 1 (one cell away, grabbable) — **expansion direction not yet settled**, see open item below |
| empty cell | single click / press+drag | currently selected point has exactly one connection (a true open tail) | continue the active subpath from that tail (click = hard corner, drag = handled+mirrored) |
| empty cell | single click / press+drag | currently selected point already has two connections (fully interior) | branch a brand-new subpath rooted at that shared point |
| empty cell | double-click | no point there | start a brand-new, fully independent subpath at this location, unconnected to anything else in the session |
| any point | delete/backspace (not a click target) | point is selected | **not yet designed** — working idea: delete the selected point; deleting every point in the session back to zero is the session's only "exit," since the session still only ever commits (possibly nothing) |

Two cells in this chart are explicitly open and need a follow-up pass with J before phase-7 locks them in:
- the double-click handle-expansion direction (toward a neighbor's tangent? a fixed axis? no neighbor case?)
- the exact delete-point input binding

Everything else above is settled design, not a guess.

## affected-encapsulations
- `domain/painter-tools/shared/`: edit
- `domain/painter-operations/shapes/`: edit
- `domain/painter-tools/individuals/vector/`: new
- `domain/painter-tools/individuals/lasso/`: edit
- `domain/painter-tools/` (root registry): edit
- `domain/painter-session/tool-state/`: edit
- `domain/painter-session/canvas-pointer/`: edit
- `domain/modules/individuals/hand-settings/`: edit
- `domain/modules/individuals/toolbox/`: edit
- `domain/user-state/user-session-state/`: edit
- `domain/tool-presets/`: edit
- `domain/tai/`: edit
- `orchestration/build-commands/`: edit

### registration ripple found by auditing every existing `PaintTool::Lasso` site (2026-11-09)
Lasso is wired into more places than the plan originally listed; vector needs the same wiring or the drift tests below fail:
- `domain/modules/individuals/toolbox/toolbox_module.rs` hardcodes a `ToolDef { tool, icon, label }` array — it does not read `ALL_TOOLS` generically, so the toolbox needs its own new `ToolDef` entry for vector.
- `domain/tai/tai.rs` is the actual default-keybinding map (separate from `registry.rs`'s informational `hotkey` field on the descriptor) — needs a `painter_select_vector` binding.
- `orchestration/build-commands/src/main.rs` drift-tests a `LIVE_PAINTER_ACTIONS` const list (fails if a declared action isn't handled or vice versa) and a `painter_control_rows()` list feeding the in-app controls/help display (`("tools", "Select Vector", "painter_select_vector")`) — both need a vector entry.
- `domain/modules/individuals/tool-presets/tool_presets_module.rs` only uses `PaintTool::Lasso` in test fixtures (picks an arbitrary existing tool to exercise preset-row behavior); it reads tool identity generically and needs no vector-specific code.

## encapsulation-details

### `domain/painter-tools/shared/`
- action: edit
- intent: add the one new cross-tool drag behavior the vector tool needs, and the one new cross-tool stroke/fill row-id helper lasso and vector both consume.
- interface delta: `DragBehavior` gains a `VectorConstruction` arm (doc'd like `TypingSession`: "a press begins a vector construction session that stays open across many presses; Enter, Escape, or switching tools commits the accumulated shape — the pointer lifecycle owns the in-progress point/handle state"); `drag_behavior("vector")` returns it. New `stroke_fill_rules.rs` (or extend `selection_rules.rs`'s re-export surface) exposing which registered tools carry the `stroke`/`fill` property rows.
- dependencies: none (leaf, same as today).
- consumers: `painter-session/tool-state/`, `painter-session/canvas-pointer/`, `painter-tools/individuals/vector/`, `painter-tools/individuals/lasso/`.
- artifacts: none
- tests: inline `#[cfg(test)]` — drift test extended for `vector`; new stroke/fill applicability test covers lasso + vector, not the other six tools.
- data: none

### `domain/painter-operations/shapes/`
- action: edit
- intent: pure vector-point/subpath geometry, parallel to `lasso.rs`. New `vector.rs`: a `VectorPoint { anchor, forward_handle: Option<Offset>, backward_handle: Option<Offset> }` (offsets stored point-relative, confirmed rigid: moving the anchor translates both handles by the same delta, no rotation/scale); a `VectorSubpath { points: Vec<VectorPoint>, closed: bool }` closed by connecting its own first and last point; a mirrored-handle constructor for the press+drag case (`backward = -forward` at authoring time only — later single-handle edits are independent); handle-toggle helpers for the double-click gesture (`collapse_handles` → zero magnitude, `expand_handles` → magnitude 1, direction TBD pending the open item in the interaction chart); a subpath-to-renderable-curve sampler so canvas-pointer/rendering can turn an open or closed point list into drawn cells. A committed shape may carry more than one subpath (the primary line plus parallel/branching authored lines) — this module models one subpath plus the even-odd fill composition across however many subpaths a session passes in; the construction session (tool-state) owns which subpaths exist and which point is selected. No session, selection, or history awareness, same as `lasso.rs`.
- interface delta: new `VectorPoint`, `VectorSubpath`, `mirrored_handle_point`, `translate_point` (rigid move), `collapse_handles`/`expand_handles`, `sample_vector_subpath` (or equivalent) functions, and an even-odd interior resolver (`vector_fill_points(subpaths, orientation) -> Vec<CellPoint>` or equivalent) that composes every closed subpath's containment together so overlapping regions subtract rather than union — may reuse lasso's point-in-polygon test per subpath internally, but the cross-subpath composition is new. 3D vector fill (punch-through-all-layers default) is called out as its own design pass, not implemented blind here.
- dependencies: `thaum-renderer-domain` geometry (`CellPoint`, `WorldAxis`, view-plane projection), `painter-operations/shapes/lasso.rs` (point-in-polygon primitive reuse), same as `lasso.rs` otherwise.
- consumers: `painter-session/tool-state/`, `painter-session/canvas-pointer/`.
- artifacts: none
- tests: inline `#[cfg(test)]` — mirrored-handle construction, rigid translation preserves handle offsets, hard-corner point (no handles) samples as a straight segment, degenerate single-point path, closing a subpath (first == last connects), open-subpath fill is never produced (fill only applies once closed), handle collapse/expand toggle, two overlapping closed subpaths subtract (even-odd) rather than union, a hole-in-a-shape (donut) case.
- data: none

### `domain/painter-tools/individuals/vector/`
- action: new
- intent: register the vector tool the same way every other tool registers — one folder, one `RegisteredTool` impl, one descriptor.
- interface delta: new `VectorTool` struct + `contract.md`; descriptor `id: "vector"`, property rows include the shared `stroke`/`fill` ids, hotkey `H`.
- dependencies: `painter-tools/` (parent registry), `painter-tools/shared/` (drag behavior, stroke/fill applicability).
- consumers: `painter-tools/registry.rs` `ALL_TOOLS`, `painter-session/tool-state/` `PaintTool::Vector`.
- artifacts: none
- tests: inline `#[cfg(test)]` — resolves its own descriptor, matching the pattern every other tool file already asserts.
- data: none

### `domain/painter-tools/individuals/lasso/`
- action: edit
- intent: lasso gains the shared `stroke`/`fill` property rows alongside its existing `fill_3d` (unchanged — `fill_3d` is depth-of-fill, not what authors the fill).
- interface delta: descriptor `property_row_ids` becomes `&["fill_3d", "stroke", "fill"]` (final ordering confirmed in phase-1).
- dependencies: `painter-tools/shared/` (stroke/fill row ids).
- consumers: `modules/individuals/hand-settings/` (row gating by `hand_declares_row`).
- artifacts: none
- tests: existing `lasso_resolves_its_own_descriptor` test updated for the new row ids.
- data: none

### `domain/painter-tools/` (root)
- action: edit
- intent: one registry line for the new tool, per the individuals contract's own note ("new tool folders join here; the registry line plus the `PaintTool` arm are the only other touches") — note that note undersells the real ripple, see the three entries below.
- interface delta: `ALL_TOOLS` gains the vector entry; `individuals/contract.md`'s `children-encapsulations` list gains `vector/`.
- dependencies: `painter-tools/individuals/vector/`.
- consumers: toolbox module, hand-settings module, tool-state parity test.
- artifacts: none
- tests: registry drift tests (tool count, id/label/hotkey parity) extended for vector.
- data: none

### `domain/modules/individuals/toolbox/`
- action: edit
- intent: the toolbox hardcodes its own `ToolDef { tool, icon, label }` array rather than reading `ALL_TOOLS`; vector needs its own entry here to actually appear in the toolbox.
- interface delta: one new `ToolDef` entry.
- dependencies: `painter-session/tool-state/` (`PaintTool::Vector`).
- consumers: the toolbox UI render path.
- artifacts: none
- tests: existing toolbox tests (tool-row rendering, selection) extended for the new entry.
- data: none

### `domain/tai/`
- action: edit
- intent: register the real default keybinding for `painter_select_vector` — `registry.rs`'s `hotkey` field is informational only; this is where a key press actually resolves to the action.
- interface delta: one new default binding entry, `H`.
- dependencies: `painter-tools/individuals/vector/` (the action name).
- consumers: input dispatch.
- artifacts: none
- tests: existing tai binding tests extended; hotkey drift tests (below) depend on this landing.
- data: none

### `orchestration/build-commands/`
- action: edit
- intent: `LIVE_PAINTER_ACTIONS` is drift-tested against the live binding map (fails if declared-but-unhandled or handled-but-undeclared) and `painter_control_rows()` feeds the in-app controls/help display — both need a vector entry or the build fails.
- interface delta: `painter_select_vector` added to `LIVE_PAINTER_ACTIONS`; `("tools", "Select Vector", "painter_select_vector")` added to `painter_control_rows()`.
- dependencies: `domain/tai/` (the binding must exist for the drift test to pass).
- consumers: the drift test suite, the in-app controls/help display.
- artifacts: none
- tests: existing drift tests (`*_resolves_to_exactly_one_action`-style) extended for vector's key.
- data: none

### `domain/painter-session/tool-state/`
- action: edit
- intent: give the vector tool session-level behavior the same way lasso's lives here today, and add the two new shared per-hand properties.
- interface delta: `PaintTool::Vector` variant (`all()`/`id()`/`from_id()` parity, `all()` becomes 9); new `VectorConstruction`-held-elsewhere accumulation API over a *session* that holds one or more `VectorSubpath`s plus exactly one selection pointer (which subpath + which point index is selected, since only the selected point's handles are ever pressable). API surface: `select_point`/`select_handle`, `place_point_from_selection` (reads the selected point's connection count to decide continue-vs-branch, per the interaction chart), `move_selected_point`/`move_selected_handle` (rigid vs. asymmetric), `close_active_subpath` (only legal when the selection is the active subpath's own open tail and the press target is that subpath's first point), `toggle_selected_point_handles` (the double-click collapse/expand), `start_independent_subpath` (double-click on empty space), `delete_selected_point` (exact trigger still open, see the interaction chart), `commit_vector_construction_for_hand` (runs the even-odd fill composition across every subpath and rasterizes the whole session as one action) — exact API shape settled in phase-1 alongside canvas-pointer's dispatch needs, each point/handle operation reading the session's live focus depth rather than a depth captured at press-start. New per-hand fields `stroke_source` / `fill_source` (tri-state: `PaintHand::Left` / `PaintHand::Right` / `None`) with `set_*_for_hand` setters mirroring `set_fill_3d_for_hand`. Resolving a source to a write goes through the existing hand-authoring seam lasso's fill already uses: `tool_state::resolved_painted_cell(existing, hand)` → `brush::write_cell`, read live at commit time (a hand's content can change mid-construction and the next-authored geometry picks that up). `None` is skipped entirely (no call, no write, existing cells untouched); `Left`/`Right` calls `resolved_painted_cell` for that hand per point, so a hand holding the clear glyph clears for free through the same blank-removal path `write_cell` already gives every other tool — no separate clear-case branch needed.
- dependencies: `painter-tools/shared/` (`DragBehavior::VectorConstruction`), `painter-operations/shapes/` (`VectorPoint`, translation/mirroring helpers).
- consumers: `painter-session/canvas-pointer/`, `modules/individuals/hand-settings/`, `user-state/user-session-state/`, `tool-presets/`.
- artifacts: none
- tests: inline `#[cfg(test)]` — parity test extended to 9 tools; new stroke/fill setter round-trips; new vector-point accumulation unit behavior (add straight point, add handled point, mirrored handle on creation, single-handle move stays asymmetric, point move translates both handles rigidly, closing a subpath only when its own open tail is selected, branching a new subpath from a selected two-connection point, selecting an arbitrary earlier point and editing it, committing a multi-subpath session as one even-odd-filled action, a hand's content change mid-session affecting only subsequently authored points).
- data: none

### `domain/painter-session/canvas-pointer/`
- action: edit
- intent: wire the vector construction session into the same dispatch loop that already owns the lasso bound and the typing session, implementing the construction-phase interaction chart's precedence exactly (point press > handle press > empty-cell create/close), including single-click-selects vs. press+drag-moves, the restricted close case (press on the active subpath's own first point while its open tail is selected), the continue-vs-branch split on empty-cell presses (by the selected point's connection count), double-click collapse/expand and double-click-to-start-an-independent-subpath, commit-on-Enter, commit-on-Escape, commit-on-tool-switch, and the live flashing preview (reusing the existing vivid/`CellGroup` preview convention already used for the lasso bound and paste preview) — rendering handles for the selected point only, every other point as a plain vertex dot, across every open subpath in the session.
- interface delta: new in-progress vector-construction state held the way `self.lasso: Option<LassoStroke>` is held today, except holding the whole multi-subpath session plus its one selection pointer rather than one path; press/drag/release arms for `DragBehavior::VectorConstruction` implementing the chart's precedence; double-click detection (not currently a canvas-pointer concept — audit how typing/other double-click-free tools would add this); Enter/Escape routed to commit the entire session (not cancel — no discard path, matching J's "commit + undo" call); audit and wire whatever already runs on tool-switch so an open session commits there too; point/handle spawn and move read the live focus depth each frame, following the Move tool's existing mid-drag depth-scroll-fold pattern (`move_drag_follows_a_mid_drag_focus_depth_scroll_to_the_new_depth`) rather than a new mechanism. The point-delete gesture and the handle-expansion direction (the chart's two open cells) are implemented only after a short follow-up design pass with J, not guessed here.
- dependencies: `painter-session/tool-state/` (vector accumulation API), `painter-operations/shapes/` (geometry/sampling), `painter-tools/shared/` (`DragBehavior::VectorConstruction`).
- consumers: the entrypoint's pointer-event dispatch (`main.rs`/`painter_modules.rs`), same as today's lasso/text wiring.
- artifacts: none
- tests: inline `#[cfg(test)]` — point press selects (not moves) on a plain click, press+drag moves it, handle press is only a valid target when its point is selected and its magnitude is nonzero, pressing the first point closes only when its own open tail is selected, pressing empty space continues vs. branches by connection count, double-click starts an independent subpath / toggles handle magnitude, Enter commits the whole session, Escape commits (not discards) the whole session, switching tools mid-construction commits the whole session, preview overlay shows handles only for the selected point across every open subpath.
- data: none

### `domain/modules/individuals/hand-settings/`
- action: edit
- intent: expose `stroke`/`fill` as one property-row shape, picked in phase-1 (leading candidate: `ListSwitchRow` per row, cycling "Left" → "Right" → "None", since the existing boolean `Matrix` row has no 3-way precedent) — "whatever is most standard and clean for scaling long term," per J, confirmed with him before this phase implements it.
- interface delta: two new rows (`stroke`, `fill`) built/handled the same way `fill_3d`'s row is today (`tool_row_used`, `hand_declares_row`, hit-handling, state read/write).
- dependencies: `painter-session/tool-state/` (`stroke_source`/`fill_source` getters/setters), `painter-tools/shared/` (which tools declare these rows).
- consumers: the properties panel UI (toolbox/hand-settings render path).
- artifacts: none
- tests: existing `fill_3d`-style hand-settings tests mirrored for `stroke`/`fill` (row only shows for lasso/vector, left/right/none cycling, state writes land on the right hand).
- data: none

### `domain/user-state/user-session-state/`
- action: edit
- intent: persist the two new per-hand fields across session save/load, exactly like `fill_3d` does today.
- interface delta: `stroke_source`/`fill_source` added to the persisted hand shape, both directions (`from` and `apply`-style round-trip).
- dependencies: `painter-session/tool-state/` (field shape).
- consumers: session file save/load.
- artifacts: none
- tests: existing `fill_3d` round-trip test pattern mirrored for the two new fields.
- data: none

### `domain/tool-presets/`
- action: edit
- intent: let presets capture/restore the two new per-hand properties, exactly like the existing `fill_3d` preset entry.
- interface delta: two new preset key entries (`stroke_source`, `fill_source`) with `get`/`set` mirroring the `fill_3d` entry's shape.
- dependencies: `painter-session/tool-state/` (field shape).
- consumers: the preset apply/capture flow.
- artifacts: none
- tests: existing preset round-trip tests extended for the two new keys.
- data: none

## phases

### phase-1 — research + plan refinement
- [ ] confirm the exact `DragBehavior::VectorConstruction` commit triggers already wired for tool-switch today (find where equipping a new tool currently clears/would need to clear in-progress state) so the canvas-pointer phase isn't guessing
- [ ] confirm the `stroke`/`fill` property-row shape with J (`ListSwitchRow` Left/Right/None is the working recommendation) before the hand-settings phase
- [ ] confirm final `property_row_ids` ordering for lasso (`fill_3d`, `stroke`, `fill`) and vector
- [ ] note: vector's hotkey is settled as `H` (J: pick any unused letter, low priority, expected to be revisited)
- [ ] note: 3D vector fill UX is an explicitly open design pass (punch-through-all-layers is the fallback) — scope phase-3/phase-7 to 2D fill first; do not block this plan's land on 3D fill being solved
- [ ] close out the construction-phase interaction chart's two open cells with J: the double-click handle-expansion direction, and the point-delete gesture
- [ ] check whether canvas-pointer's dispatch loop already has any double-click detection anywhere in the codebase to reuse before inventing one
- [ ] refine this plan until every middle phase below is executable as written

### phase-2 — `domain/painter-tools/shared/`
- [ ] add `DragBehavior::VectorConstruction` + `drag_behavior("vector")`
- [ ] add the stroke/fill row-applicability helper (lasso + vector only)
- [ ] extend drift tests for the new variant and the new helper

### phase-3 — `domain/painter-operations/shapes/`
- [ ] add `vector.rs`: `VectorPoint`, `VectorSubpath` (open/closed), mirrored-handle construction, rigid point-translation, handle collapse/expand toggle, subpath sampling
- [ ] 2D fill: even-odd composition across every closed subpath in a session, reusing lasso's point-in-polygon test as an internal primitive where it fits
- [ ] tests: mirroring, rigid translation, hard-corner (no-handle) sampling, degenerate single-point path, closing a subpath, open-subpath fill never fires, handle collapse/expand, overlapping subpaths subtract (donut/hole case)

### phase-4 — `domain/painter-tools/individuals/vector/` + full registration ripple
- [ ] new folder, `VectorTool` descriptor + `contract.md`, registry line, individuals `children-encapsulations` entry
- [ ] descriptor test mirroring every other tool's resolution test
- [ ] `domain/modules/individuals/toolbox/`: new `ToolDef` entry (toolbox does not read `ALL_TOOLS` generically)
- [ ] `domain/tai/`: default `painter_select_vector` keybinding
- [ ] `orchestration/build-commands/`: `LIVE_PAINTER_ACTIONS` + `painter_control_rows()` entries so drift tests pass

### phase-5 — `domain/painter-tools/individuals/lasso/`
- [ ] add `stroke`/`fill` to lasso's `property_row_ids`
- [ ] update lasso's descriptor test

### phase-6 — `domain/painter-session/tool-state/`
- [ ] `PaintTool::Vector` variant + full parity (`all()`, `id()`, `from_id()`)
- [ ] vector construction API over a multi-subpath session with one selection pointer (select point/handle, place-from-selection with continue-vs-branch, move selected point/handle, close active subpath, toggle selected point's handles, start an independent subpath, delete selected point, commit the whole session for hand)
- [ ] `stroke_source`/`fill_source` per-hand fields + setters, resolved live through `resolved_painted_cell` → `write_cell` at commit time
- [ ] tests per the encapsulation-details list above

### phase-7 — `domain/painter-session/canvas-pointer/`
- [ ] in-progress multi-subpath vector-construction state (session + selection pointer) held alongside the existing lasso/typing state
- [ ] press/drag/release/double-click dispatch for `DragBehavior::VectorConstruction` implementing the interaction chart's precedence exactly (point > handle > create/close)
- [ ] Enter/Escape/tool-switch commit wiring for the whole session (no discard path)
- [ ] live flashing preview: handles render only for the selected point, every other point a plain vertex dot, reusing the existing vivid/`CellGroup` preview convention, across every open subpath
- [ ] tests per the encapsulation-details list above

### phase-8 — `domain/modules/individuals/hand-settings/`
- [ ] add the confirmed `stroke`/`fill` row shape, gated to lasso/vector
- [ ] tests mirroring the existing `fill_3d` row tests

### phase-9 — `domain/user-state/user-session-state/` + `domain/tool-presets/`
- [ ] persist `stroke_source`/`fill_source` in session save/load
- [ ] register both fields in the tool-presets get/set table
- [ ] tests mirroring the existing `fill_3d` round-trips in both encapsulations

### phase-final — implementation closure + tests + approval + commit
- [ ] verify every planned task above is complete
- [ ] run the full workspace test suite (`cargo test -p thaum-renderer-domain --lib`, `cargo test -p thaum-painter-entrypoint` or the relevant crate scopes touched)
- [ ] manual pass in the running painter: place a hard-corner vector point, a handled point, move a handle asymmetrically, move a vertex and confirm both handles translate rigidly, scroll depth mid-drag while spawning/moving a point or handle and confirm it lands at the new depth, select an earlier non-active point and edit it, close a subpath only when its own open tail is selected (confirm it refuses otherwise), branch a new subpath from a selected two-connection point, double-click empty space to start an independent subpath, double-click a point to collapse then expand its handles, draw two overlapping closed subpaths and confirm the overlap subtracts (a hole/donut), start a second subpath mid-session and confirm both commit together as one undo step, commit via Enter, commit via Escape, commit via tool-switch, undo in one `z` press; set lasso and vector stroke/fill to Left/Right and confirm the authored content matches the selected hand's graphic/color/weight (including clearing when that hand holds the clear glyph), swap a hand's graphic/color mid-construction and confirm only subsequently authored points pick it up, and confirm `None` leaves existing cells untouched
- [ ] review with J and record approval
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- project-git-commit: false
- shape tools (2D and 3D) and stroke size are explicitly deferred — they reuse this plan's stroke/fill plumbing once vector lands, but are out of bounds for this plan.
