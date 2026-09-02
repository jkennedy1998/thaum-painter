# third pass encapsulation review

## superseded note (26-08-31)
This review's `domain/painter-document/modules/` verdict (below, and in the "module concept added" addendum) is stale: that seam has been removed and its duties folded into `domain/painter-document/groups/`, which now maps directly 1:1 to renderer `CellGroup`s. See `context/module-concept-audit.md`'s "superseded" section.

## purpose
Review the current thaum-painter encapsulations after the old-feature ownership pass and decide which seams are ready as-is, which seams should split now, and which seams should intentionally wait for implementation pressure.

## verdict summary

### ready as-is
- `domain/file/`
- `domain/file/storage/`
- `domain/file/import-export/`
- `domain/file/migration/`
- `domain/rendering/camera/`
- `domain/painter-document/modules/`
- `domain/painter-document/groups/`
- `domain/painter-document/properties/`
- `domain/painter-session/commands/`
- `domain/painter-session/history/`
- `domain/painter-session/selection/`
- `domain/painter-session/tool-state/`
- `domain/painter-operations/brush/`
- `domain/painter-operations/shapes/`
- `domain/painter-operations/fill/`
- `domain/painter-operations/text/`
- `domain/painter-operations/image-import/`

### should split now
- none

### should intentionally wait
- `domain/file/manifest/`
- `domain/rendering/render-space/`
- `domain/painter-document/timing/`
- `domain/painter-session/clipboard/`

## why nothing should split immediately
The current structure is finally clean enough that splitting more right now would mostly be speculative. The risky move would be inventing sub-seams before there is implementation pressure proving where the actual fracture lines are. The old system's biggest problem was fusion, but premature sub-boundaries can also create fake clarity.

## seam-by-seam review

### ready as-is

#### `domain/painter-document/modules/`
Good top seam, added after this review. Module identity/order/placement is the top-level authored unit and deserves its own editing-facing seam separate from intra-module group lookups; see `context/module-concept-audit.md`.

#### `domain/file/`
Good top seam. Saved truth is clearly separated from renderer handoff and live session state.

#### `domain/file/storage/`
Good boundary. Persistence policy is distinct from file shape.

#### `domain/file/import-export/`
Good boundary. Transfer wrappers are distinct from canonical file truth.

#### `domain/file/migration/`
Good boundary. Legacy compatibility should stay isolated.

#### `domain/rendering/camera/`
Good boundary. Live camera intent is clearly separated from saved defaults and renderer internals.

#### `domain/painter-document/groups/`
Good boundary. Editing-facing group views are a real seam and not too broad yet.

#### `domain/painter-document/properties/`
Good boundary. Property lookup/helpers deserve their own editing-facing seam.

#### `domain/painter-session/commands/`
Good boundary. Command vocabulary should stay separate from history and pure ops.

#### `domain/painter-session/history/`
Good boundary. Undo/redo is clearly session policy.

#### `domain/painter-session/selection/`
Good boundary. Both plane and world selection can still fit here without immediate pain.

#### `domain/painter-session/tool-state/`
Good boundary. Active tool state is distinct from pure ops and persistence.

#### `domain/painter-operations/*`
All five operation seams feel right already. They match the old feature clusters without re-fusing runtime and UI concerns.

### intentionally wait

#### `domain/file/manifest/`
Do not split yet.

Why wait:
- we know it will hold content sections, metadata, time assets, and saved camera defaults
- but we do not yet know the final stored top-level section shape
- splitting into `content/`, `metadata/`, `time-assets/`, or `saved-camera/` now would likely be guesswork

Trigger to split later:
- once the actual saved shape is drafted and sample files show one section changing independently from others

#### `domain/rendering/render-space/`
Do not split yet, but expect it to split first.

Why wait:
- it is broad, but still conceptually one thing today: renderer handoff assembly
- the correct child seams depend on implementation pressure from preview, timing, and scene assembly code

Likely split later:
- `scene/`
- `timing/`
- `preview/`
- `visible-stack/`

Trigger to split later:
- once one render-space implementation has to own both renderer-ready scene assembly and editor-preview-specific derivation in the same place

#### `domain/painter-document/timing/`
Do not split yet.

Why wait:
- timing is definitely important, but the current top seam is still understandable
- saved time assets already have a different owner in `domain/file/manifest/`
- render-facing timing mapping already has a different owner in `domain/rendering/render-space/`
- so the dangerous split is already conceptually present even if not represented by more directories yet

Likely split later:
- editing timing lookup
- breath playback helpers
- authored time-asset convenience views

Trigger to split later:
- once timeline editing and time-asset authoring become separate implementation surfaces

#### `domain/painter-session/clipboard/`
Do not split yet.

Why wait:
- it currently still reads as one live authoring concern
- we know it may later contain state, codecs, and paste-preview behavior, but not all of that needs separate ownership on day one

Likely split later:
- `buffer/`
- `codecs/`
- `paste-preview/`

Trigger to split later:
- once clipboard payload encoding and paste-preview state begin changing independently

## practical rule for future iterations
Only split a seam when at least one of these becomes true:
- one child would have different tests than the parent
- one child would have different consumers than the parent
- one child would have a different lifecycle than the parent
- one child would tempt mixed ownership if left inside the parent

## current recommendation
- keep the current tree
- do not add more child seams yet
- next use implementation-ready examples to pressure-test `domain/file/manifest/` and `domain/rendering/render-space/`
- expect `render-space/` to be the first seam that truly proves it wants children

## addendum: module concept added
Pressure-testing `domain/rendering/render-space/` against `thaum-renderer`'s own design truths (not just the saved-file example) surfaced a real missing seam: `domain/painter-document/modules/`, plus `document.modules[]` in `domain/file/manifest/`. This was not a case of splitting too early — it was a genuinely missing top-level concept the old `module_position_storage.ts` already proved out. See `context/module-concept-audit.md` for the full reasoning and what changed in the manifest and render-space examples.
