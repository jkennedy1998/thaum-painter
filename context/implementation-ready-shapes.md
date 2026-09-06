# implementation-ready shape notes

## purpose
Pin the first concrete saved-file and render-space examples so future implementation work has stable targets to build toward.

## current example artifacts
### saved file manifest
- `thaum-painter/domain/file/manifest/example-thaum-painter-file-v1.json`
- `thaum-painter/domain/file/manifest/example-thaum-painter-file-v1.md`
- `thaum-painter/domain/file/manifest/schema-thaum-painter-file-v1.json`

### render-space handoff
- `thaum-painter/domain/rendering/render-space/example-render-space-v1.json`
- `thaum-painter/domain/rendering/render-space/example-render-space-v1.md`
- `thaum-painter/domain/rendering/render-space/schema-render-space-v1.json`

## intent
- the file example is the first concrete statement of saved authored truth.
- the render-space example is the first concrete statement of transient renderer handoff truth.
- the two examples should stay aligned but should not collapse back into one broad shape.

## expected use
- future parser/serializer work should validate against the saved file example.
- future manifest validation can start from `schema-thaum-painter-file-v1.json`.
- future render translation work should validate against the render-space example.
- future render-space validation can start from `schema-render-space-v1.json`.
- future contract revisions should update these artifacts when ownership boundaries change.
