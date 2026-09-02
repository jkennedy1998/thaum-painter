# example alignment review

## purpose
Validate the broader thaum-painter contracts against the first concrete saved-file and render-space examples.

## addendum: module restructuring
The examples originally validated here modeled one flat `document.groups[]` list with each group mapping 1:1 to its own renderer cell-group. That has since been corrected: `document.modules[*].groups[]` now nests groups one level deeper, and render-space emits one `cell_groups[]` entry per module with intra-module groups composited into it, matching `thaum-renderer`'s own truth that a `CellGroup` is "an entire module and relevant contents." See `context/module-concept-audit.md` for the full reasoning. The confirmed boundaries below still hold; only the concrete shape of `domain/file/manifest/` and `domain/rendering/render-space/`'s examples changed.

## addendum 2: module restructuring reversed (26-08-31)
The module wrapper described directly above has itself been reversed. `document.groups[]` is flat again, one group mapping directly to one renderer `CellGroup` — same as the original pre-addendum-1 shape, but this time it's the intentional, permanent one. See `context/module-concept-audit.md`'s "superseded" section for why (the renderer's camera turned out to be universal, not module-local, so the bundling reason from addendum 1 no longer applies). The v1 example/schema JSON and `manifest.rs`/`render_space.rs` still need a code pass to catch up to this.

## validated examples
- `/home/j/Repos/thaum-painter/domain/file/manifest/example-thaum-painter-file-v1.json`
- `/home/j/Repos/thaum-painter/domain/rendering/render-space/example-render-space-v1.json`

## result
The current contract tree still holds.

### confirmed boundaries
- `domain/file/manifest/` still reads correctly as canonical saved truth.
- `domain/painter-document/` still reads correctly as an editing-facing view over saved truth rather than the file owner.
- `domain/rendering/camera/` still reads correctly as the place where saved camera defaults and live overrides are reconciled before handoff.
- `domain/rendering/render-space/` still reads correctly as transient renderer handoff assembly rather than saved file ownership.
- `domain/file/import-export/` still reads correctly as wrapper/profile policy around the canonical file and render-space output.
- `domain/file/storage/` still reads correctly as persistence policy rather than file-shape ownership.

## pressure findings
### no immediate seam split required
The examples did not reveal a seam that must split now.

### likely first future pressure point
`domain/rendering/render-space/` still looks like the first place likely to want children once preview-specific derivation and scene assembly both become real code.

### next likely pressure point
`domain/file/manifest/` may want child seams later if metadata, document content, time assets, and saved camera defaults begin changing independently in implementation or tests.

## implementation guidance
- parser/serializer work should anchor to the saved-file example.
- document-view helpers should treat the saved-file example as input, not as their owned schema.
- camera resolution should treat `saved_camera_defaults` as baseline input and live focus/target state as override input.
- render translation should emit shapes equivalent to the render-space example after timing/property resolution.
