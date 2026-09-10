# project plan — Thaum Painter release status + download page

## implementation-rules
- Update checklist states while working.
- Keep compatibility/versioning separate from multiplayer: release status never changes connection admission.
- Reuse the renderer command-bar and tooltip seams; do not special-case a HUD control in Painter.
- The browser action opens the download page only. It never downloads, replaces, or installs an app.

## status key
- `[ ]` not done
- `[+]` implemented
- `[#]` tested

## pre-implementation-note
Thaum Painter currently ships v0.1.2 as portable Linux, Windows, and unsigned Mac archives through GitHub Releases. The site has those three direct downloads only in its Development portfolio entry; `/thaum-painter/` does not exist. The live Session panel and `SESSION_PROTOCOL_VERSION` remain deliberately out of scope: J wants old compatible builds to keep connecting. The new interaction is a global bottom command-bar version button that passively assesses whether this release is current, colors itself honestly, supplies a tooltip after the shared dwell, and opens the one website download page on click.

## affected-encapsulations
- `/home/j/Repos/thaum-renderer/domain/command-bar/`: edit
- `thaum-painter/orchestration/build-commands/`: edit
- `/home/j/Repos/jartanddesign-website/thaum-painter/`: new
- `/home/j/Repos/jartanddesign-website/js/components.js` and `css/style.css`: edit (optional source-defined release-notes rendering)
- `/home/j/Repos/jartanddesign-website/source/development/`: edit

## encapsulation-plans
- `/home/j/Repos/thaum-renderer/domain/command-bar/`: `plans/home-j-repos-thaum-renderer-domain-command-bar-release-status-plan.md`
- `thaum-painter/orchestration/build-commands/`: `plans/home-j-repos-thaum-painter-orchestration-build-commands-release-status-plan.md`
- `/home/j/Repos/jartanddesign-website/thaum-painter/`: `plans/home-j-repos-jartanddesign-website-thaum-painter-plan.md`

## encapsulation-details
### `/home/j/Repos/thaum-renderer/domain/command-bar/`
- action: edit
- intent: Let any command-bar button carry its deliberate UI color and hover tooltip through the existing shared tooltip seam.
- interface delta: `CommandBarButton` gains optional presentation/tooltip metadata; `CommandBar` exposes hoverable button hotspots using its real current layout.
- dependencies: `thaum-renderer/domain/modules/shared/tooltip/` — reuse `Hotspot`; `UiColorRole` — color remains palette-owned.
- consumers: Thaum Painter supplies the release-status button plus standard `FILE` and `MODULES` tooltips; all other buttons retain current behavior unless configured.
- artifacts: none
- tests: inline command-bar tests for default compatibility, per-button palette role, and hotspot geometry after layout/menu changes.
- data: none

### `thaum-painter/orchestration/build-commands/`
- action: edit
- intent: Own passive release-manifest checking, the status-to-button mapping, opening the canonical page, and release-time publication of the canonical page facts.
- interface delta: internal `ReleaseStatus`/manifest receiver and a `version:downloads` command-bar action; no multiplayer wire or session-panel delta.
- dependencies: renderer command bar/tooltip capability; `https://jartanddesign.com/thaum-painter/release.json`; existing website/GitHub publishing seams.
- consumers: the Painter frame loop renders the button; users click it to visit the page; `release-thaum-painter` publishes the new site version, generated GitHub release notes, manifest, and verified stable download-link facts only after GitHub Release asset verification.
- artifacts: generated site `thaum-painter/release.json`; no local update cache or installer artifact.
- tests: Rust unit tests for manifest parsing and SemVer status resolution; action-routing/browser-URL test seam; release-script static/publisher checks.
- data: none

### `/home/j/Repos/jartanddesign-website/thaum-painter/`
- action: new
- intent: Create the only public Thaum Painter download/update destination in the existing portfolio-page visual system, including the current release's GitHub-generated notes.
- interface delta: static `/thaum-painter/` route and `release.json` manifest with the stable release version and canonical page URL; the existing generic portfolio source renderer accepts an optional `release notes` section without changing slices that omit it.
- dependencies: shared `css/style.css`, `js/components.js`, and the generated portfolio source convention; GitHub `releases/latest/download` assets.
- consumers: Painter’s version button; all download links; the Development portfolio entry links here instead of owning downloads.
- artifacts: `thaum-painter/index.html`, generated `source/thaum-painter/entries.js`, source entry, and `thaum-painter/release.json`.
- tests: website source generation and static link/route assertions; manual responsive/hover pass.
- data: none

### `/home/j/Repos/jartanddesign-website/source/development/`
- action: edit
- intent: Retain the portfolio project card while routing it to the canonical Thaum Painter page.
- interface delta: one `Thaum Painter` page link replaces the three platform archive links.
- dependencies: `/thaum-painter/` route.
- consumers: Development portfolio visitors.
- artifacts: regenerated `source/development/entries.js`.
- tests: source build plus generated-entry assertion.
- data: none

## phases
### phase-1 — architecture alignment + encapsulation ordering
- [+] confirm the three final release states: `up-to-date`, `out-of-date`, and `cannot-assess`.
- [+] record the settled rule that app release status is not multiplayer compatibility and does not touch `SESSION_PROTOCOL_VERSION` or Session-panel code.
- [+] lock the public manifest contract: stable semantic version plus canonical page URL, served only from `/thaum-painter/release.json`.
- [+] confirm the website page is published before an app release can report that release as available.
- [+] verify all plan links and the website source/generation paths before implementation begins.

### phase-2 — plan encapsulation: `/home/j/Repos/jartanddesign-website/thaum-painter/`
- [+] confirm the new route uses the existing portfolio-page shell, shared header/footer, typegrid, palette, and interactive link treatment.
- [+] confirm its one source entry contains the product summary, current version, GitHub-generated release notes, the three existing platform download links, and concise platform instructions.
- [+] confirm `release.json` is static, minimal, and has no artifact URL that the app could automatically download.
- [+] record the linked website plan’s artifact, test, and release-publisher handoff.
- [+] record why this comes first: it establishes the canonical destination and manifest contract for the app.

### phase-3 — plan encapsulation: `/home/j/Repos/thaum-renderer/domain/command-bar/`
- [+] confirm generic button presentation and tooltip metadata do not alter existing buttons unless opted in; configure the root `FILE`, `MODULES`, and version buttons as the first standard bottom-bar tooltip consumers.
- [#] confirm hotspots are computed from the exact visible button layout, including menu nesting and scrolling.
- [+] confirm shared tooltip dwell, placement, and rendering remain owned by the existing tooltip seam.
- [+] record the linked renderer plan’s interface, test, and consumer expectations.
- [+] record why this precedes Painter wiring: Painter must consume rather than reimplement button coloring/tooltip geometry.

### phase-4 — plan encapsulation: `thaum-painter/orchestration/build-commands/`
- [#] confirm the checker runs asynchronously after app startup, has a bounded timeout, and maps any fetch/parse/version failure to `cannot-assess` without interrupting painting.
- [#] confirm labels, palette roles, and tooltip copy exactly follow J’s three-state direction.
- [#] confirm click opens `https://jartanddesign.com/thaum-painter/` in the system browser and does nothing else.
- [#] confirm `release-thaum-painter` validates version alignment, verifies the GitHub Release assets, then updates/pushes the website’s page version, GitHub-generated release notes, manifest, and generated output.
- [+] record the linked Painter plan’s dependency, artifact, test, and ordering expectations.

### phase-5 — final verification + repo-rule sweep + git commit
- [+] verify every touched target has the linked implementation plan and the site route is the only download destination.
- [ ] run command-bar, Painter, and website-generation/link tests; manually exercise each release state and the browser action.
- [ ] run final encapsulation/repo-rule review across Renderer, Painter, and website scope.
- [#] verify a release command cannot publish a manifest version before matching GitHub Release assets exist.
- [ ] git commit each repo only if approved.

## post-implementation-notes
- implementation-status: core website route/manifest/release-notes presentation, generic command-bar tooltip/color support, passive Painter check/browser action, and release publisher are implemented locally on 2026-09-10.
- verification-status: Painter’s 27 entrypoint tests (including prerelease SemVer), Renderer’s 8 command-bar tests, website generation/static-link/release-notes assertions, shell syntax, and the release-version mismatch guard passed. Existing unrelated compiler warnings remain.
- remaining-before-finish: deploy the website route so the live manifest stops returning 404, manually inspect the three tooltip cards/button colors and browser action in a running Painter, and exercise a future real three-asset release (including its generated GitHub release notes) without publishing one merely for this feature.
- audit-2026-09-10: reconciled the implementation with the settled spec; added the previously omitted page release-notes path, made the release command consume GitHub’s generated notes only after asset verification, and corrected the checklist to distinguish verified work from the three intentionally deferred live/manual checks.
- repository-state-2026-09-10: Painter implementation files were included in existing commit `90e7dee` alongside unrelated custom-gizmo tooltip work; do not amend/rewrite that mixed commit without J’s direction. Renderer and website implementation work remain uncommitted; the three plan files are current audit-only Painter working-tree edits.
- plan-finished: false
- encapsulation-git-commit: false
