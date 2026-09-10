# encapsulation plan — jartanddesign.com Thaum Painter download page

linked project plan: `plans/project-thaum-painter-release-status-and-download-page-plan.md` (phase-2)

## implementation-rules
- Update checklist states while working.
- Reuse the existing portfolio-page shell, source-entry convention, shared header/footer, components, typegrid, and link interactions.
- The page is the public download destination; it does not become an installer/updater service.

## status key
- `[ ]` not done
- `[+]` implemented
- `[#]` tested

## pre-implementation-note
The site already renders Development, Illustration, Design, and Sketchbook through directory routes, `data-portfolio-page`, generated `source/<page>/entries.js`, and shared `components.js`/`style.css`. Thaum Painter is currently one Development slice with three direct GitHub asset links. This work creates an equivalent styled `/thaum-painter/` route owning those links and the current GitHub-generated release notes, plus the existing site convention’s root `thaum-painter.html` companion, then converts the Development slice to one link to the new route.

## target-encapsulation
- `/home/j/Repos/jartanddesign-website/thaum-painter/` and `/home/j/Repos/jartanddesign-website/thaum-painter.html`: new
- `/home/j/Repos/jartanddesign-website/js/components.js` and `css/style.css`: edit only to render optional source-defined `release notes` and static-description fields through the existing portfolio slice language; all other slices remain unchanged.

## artifacts
- `thaum-painter/index.html` — canonical directory-route shell using `data-portfolio-page="thaum-painter"`.
- `thaum-painter.html` — root-file companion shell, matching every existing top-level portfolio page and using root-relative asset paths.
- `source/thaum-painter/2026/1/entry.md` — source-of-truth product slice, including the release-notes section published from the verified GitHub Release.
- `source/thaum-painter/build-entries.mjs` and generated `source/thaum-painter/entries.js` — existing page-source generation shape.
- `thaum-painter/release.json` — static public manifest: current semantic version and canonical page URL only.
- edited `js/components.js` and `css/style.css` — optional generic portfolio release-notes and non-expandable static-description rendering; blank single-media entries no longer produce a broken-image title artifact; no page-specific parallel UI.
- edited `source/development/2026/1/entry.md` and regenerated `source/development/entries.js` — one product-page link, no platform downloads.

## tests
- `node build-entries.mjs` successfully rebuilds both source-entry outputs.
- static assertion/manual inspection: both route shells have the shared page key and appropriate asset paths; the canonical route has exactly three platform asset links, rendered release notes, valid manifest, and Development has one `/thaum-painter/` link.
- responsive/hover manual pass in a browser.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [+] inspect the existing Development and Sketchbook source/build patterns and preserve generated-versus-source ownership.
- [+] record the canonical route and manifest contract consumed by Painter: `https://jartanddesign.com/thaum-painter/release.json` with version/page only.
- [+] record that the three GitHub `releases/latest/download` links move wholesale from Development to this page and that the verified GitHub Release body is rendered as its `release notes` section.
- [+] define the source files the Painter release command may update and preserve all other portfolio content.

### phase-2 — page route + shared presentation
- [+] add the new directory route, root-file companion, source directory, and generator following the existing portfolio-page pattern.
- [#] extend the generic portfolio source renderer only enough to render an optional `release notes` field; pages without one remain visually/behaviorally unchanged.
- [#] author one Thaum Painter product slice in the existing visual language with product description, compiled current version, rendered release notes, and the three interactive platform links.
- [+] include concise extraction/run guidance and the existing Mac unsigned/Gatekeeper note without inventing an auto-install flow.
- [+] use shared components/styles; add page-local styling only if the shared slice system demonstrably cannot express a needed product fact.

### phase-3 — canonical download ownership + manifest
- [#] remove Linux/Windows/Mac archive links from the Development entry and replace them with one `Thaum Painter` page link.
- [#] write a valid minimal release manifest matching the initial published Painter version and canonical page URL.
- [#] run the root generator so checked-in `entries.js` outputs agree with source entries.
- [ ] confirm the live route and manifest resolve after website publication, not only from a local file path.

### phase-4 — validation + repo-rule sweep + git commit
- [ ] validate desktop and narrow layouts, the shared header/footer, link hover behavior, and all three download URLs.
- [#] validate the manifest against the app’s parser and confirm it cannot direct an automatic artifact download.
- [#] verify generated files have no hand-only drift from their source entries.
- [ ] run available website checks and review repository conventions.
- [ ] git commit if approved.

## post-implementation-notes
- audit-2026-09-10: the page's optional release-notes field now renders through the shared portfolio source renderer and styles, leaving every slice without that field unchanged. Source/generator/static assertions pass; browser responsive/hover inspection and live deployment remain deferred.
- route-completeness-2026-09-10: added `thaum-painter.html` as the root-level companion to `thaum-painter/index.html`, matching the established `development.html`/`development/index.html` shape so the page is discoverable in either top-level form.
- content-refinement-2026-09-10: removed the blank-media broken-image artifact, made J’s supplied Painter copy static rather than expandable, set `release notes — v0.1.2 first public release`, and applies dark `#120A1A` consistently to the Painter slice, page remainder, and viewport background. Static copy is bounded to the normal 52-character measure and wraps safely.
- plan-finished: false
- encapsulation-git-commit: false
