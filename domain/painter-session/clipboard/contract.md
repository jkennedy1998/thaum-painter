# /home/j/Repos/thaum-painter/domain/painter-session/clipboard

## purpose
Own live clipboard and copy-buffer semantics for painter session workflows.

## owns
- per-user in-app copy-buffer state (`UserClipboards`, one `WorldCopyData` per `user_id` for multiplayer)
- the 3D world copy payload shape (`WorldCopyData`: anchor + center + sparse cells offset from the anchor)
- copy from the selection surface (world selection first, plane selection fallback) out of the live canvas
- paste target resolution (`paste_points` placing copy offsets relative to the copy's center cell at a paste anchor)
- the `THAUM3D:` + JSON OS-clipboard text codec (`encode_os_clipboard` / `decode_os_clipboard`) with its own serde payload mirror of painted cells
- clipboard mode notes for plain text and world-aware selections (plain-text paste decode is future work)

## does not own
- OS clipboard transport details (the caller writes/reads the actual clipboard text)
- selection ownership (consumes `PainterSelection` reads and `world_mut`)
- the document commit path: paste callers stage into the canvas and commit through `session_document`'s stroke seam so a paste is one undo step like any stroke
- canonical import/export ownership

## children-encapsulations
- none

## contents
- `contract.md`
  - clipboard contract
- `copy_paste.rs`
  - `WorldCopyData`, `UserClipboards`, copy-from-selection, paste-point resolution, and the `THAUM3D:` OS-clipboard codec

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-session/selection/`
- `/home/j/Repos/thaum-painter/domain/painter-session/paint-color/`
- `/home/j/Repos/thaum-painter/domain/painter-operations/brush/`
- `/home/j/Repos/thaum-painter/domain/file/storage/` (material name mapping shared with persisted cells)

## exposed interfaces
- none (in-crate module consumed by painter-session and the entrypoint)

## interface consumers
- painter-session
- orchestration/entrypoint (copy/paste keybinding wiring)
- future multiplayer surfaces consuming per-user buffers

## artifacts
- future clipboard payload fixtures

## tests
- inline `#[cfg(test)]` in `copy_paste.rs`
  - light
  - validates 3D copy across z layers relative to the min-corner anchor, world-selection priority over plane, sparse unpainted-cell skipping, paste-anchor placement, per-user buffer isolation, `THAUM3D:` payload roundtrip, and foreign-clipboard-text rejection

## data
- none

## notes
- source-of-truth from J: the clipboard must be 3D — payload cells carry full x/y/z offsets, mirroring the old system's `world_selection.ts` `WorldCopyData` (`THAUM3D:` prefix), not the old 2D `copy_paste.ts` grid.
- source-of-truth from J: clipboards are user-based for multiplayer — one buffer per `user_id`; the OS-clipboard codec is the shared cross-session bridge so users see pasted structures emerge in their own session.
- source-of-truth from J: paste always uses the user's **own** buffer — users handle their own copy/paste. A `THAUM3D:` payload found on the OS clipboard (e.g. copied in another session) imports into the own buffer on first paste; other users' buffers are never read.
- source-of-truth from J: the paste cursor sits in the **middle** of the copied content, empty tiles included — the copy records the bounds-center cell of the copied region (odd extents center exactly, even extents round away from the min) and `paste_points` lands that center on the cursor.
- source-of-truth from J (unified empty-cell rule): copies never carry authored blanks — `copy_from_canvas` skips space-glyph cells, and `paste_points` places nothing for blank payload cells (legacy payloads included).
- entrypoint wiring: `C` copies the selection (own buffer + OS clipboard mirror). Pasting is the **stamp tool** (source-of-truth from J, 2026-09-04): `V` equips Stamp instead of pasting directly, and equipping it imports a `THAUM3D:` OS-clipboard payload into the own buffer — after that the stamp works from the own buffer only. A press places the copied structure at the click cell through the hand's resolved painted cell (channel locks respected, selection-gated), staged in `canvas-pointer` and committed as one undo step; hovering shows a two-phase vivid flash of the paste area.
- copies are sparse: pasting writes only copied cells and never erases the paste target region.
- plain-text paste (foreign ASCII text becoming glyphs at the paste anchor with hand-state styling) is deliberately future work; `decode_os_clipboard` rejects non-`THAUM3D:` text so callers can fall back cleanly.
