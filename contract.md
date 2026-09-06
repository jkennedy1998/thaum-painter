# thaum-painter

## purpose
Own the thaum painter as its own repo-shaped authoring boundary for ASCII scene creation, saved painter documents, renderer export, and future game-facing art workflows.

## owns
- the repo-level thaum-painter contract boundary
- painter-owned file, rendering, document, session, and tool-operation design truth
- the jobo-style encapsulated organization for painter work
- painter-side save/load semantics and painter-to-painter transfer semantics
- painter-side authoring metadata that should not leak into renderer core format

## does not own
- renderer-owned ASCII 3d matrix intake format
- renderer composition, camera, post-effects, or surface boot
- game-specific runtime logic beyond import/export seams

## children-encapsulations
- `domain/`
  - default
- `orchestration/`
  - default
- `tools/`
  - default
- `workers/`
  - default

## contents
- `contract.md`
  - repo contract
- `domain/`
  - painter-owned semantic boundaries
- `orchestration/`
  - app boot composition seams
- `tools/`
  - reusable low-level painter helpers
- `workers/`
  - bounded async/background seams
- `context/`
  - roadmap and design-truth notes; not an encapsulation, matching how `thaum-renderer/` treats its own `context/`
- `plans/`
  - per-encapsulation and project-level implementation plans; not an encapsulation

## dependencies
- `thaum-renderer/`

## exposed interfaces
- none

## interface consumers
- humans and operators shaping thaum painter
- future painter implementation surfaces
- future game/app consumers of painter exports

## artifacts
- none

## tests
- none

## data
- none

## notes
- source-of-truth from J: painter should mirror jobo-style architecture with repo-local encapsulations.
- source-of-truth from J: the first pass needed more encapsulation than the old software had.
- source-of-truth from J: `domain/file/` should manifest and store what the renderer does not have.
- source-of-truth from J: `domain/rendering/render-space/` should be how render file or live state sends data to `thaum-renderer`.
- source-of-truth from J: renderer already owns one intentional standard way into the renderer for ASCII 3d matrix data, and painter should build with that instead of inventing a competing core format.
- prefer one clear renderer handoff seam instead of parallel bridge concepts that overlap with `domain/rendering/render-space/`.
