# encapsulation plan — Painter release-status button + publisher

linked project plan: `plans/project-thaum-painter-release-status-and-download-page-plan.md` (phase-4)

## implementation-rules
- Update checklist states while working.
- Do not alter Session-panel state, session protocol, or connection admission.
- The app assesses availability but never downloads, self-updates, or prompts an installer.

## status key
- `[ ]` not done
- `[+]` implemented
- `[#]` tested

## pre-implementation-note
The Painter entrypoint owns the live command bar and the release command. It exposes only `FILE` and `MODULES` today. J settled that the replacement is a direct version-number button next to them: it is Bright when the hosted manifest matches the compiled app, Vivid when the manifest is newer, and Medium whenever availability cannot be assessed. Its shared hover tooltip explains the state; its click opens the canonical page only.

## target-encapsulation
- `thaum-painter/orchestration/build-commands/`: edit

## artifacts
- website-owned `thaum-painter/release.json` plus the page's displayed version and generated GitHub release notes are published by `release-thaum-painter`; Painter does not create a local artifact/cache for update state.

## tests
- inline/new Rust tests for semantic-version comparison, valid/malformed manifest handling, and each three-state button presentation/action mapping.
- release command test/proof using a controlled website worktree or equivalent static assertions that the source entry's version/release notes/stable platform links, manifest, and generated entries are updated only after release asset verification.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [+] retain the recorded J truths in `truth.md`: old compatible versions connect, no auto-update, global command-bar status replaces Session-panel work, and release owns public page facts.
- [+] document the manifest’s exact minimal shape and the three status states in the local build/release contract.
- [+] verify the renderer command-bar plan has landed or explicitly identify its exposed configuration/hotspot surface before consuming it.
- [+] record page URL, timeout/failure policy, browser action, test, and no-data expectations.

### phase-2 — passive release assessment
- [#] add a narrow release-status owner in this encapsulation: compiled version, manifest decode, semantic comparison, and `cannot-assess` failure state.
- [+] begin its HTTPS fetch without delaying first render; use a short bounded timeout and receive the result safely on the UI thread.
- [#] treat invalid JSON, invalid SemVer, unavailable network, non-success HTTP, and an older/equal malformed policy as `cannot-assess` or `up-to-date` only where the manifest is valid; never guess success.
- [#] cover `up-to-date`, `out-of-date`, unavailable, malformed, and prerelease policy in pure tests.

### phase-3 — command-bar interaction
- [#] add the direct version-number button beside `FILE` and `MODULES`, never as a nested menu or Session-panel row; configure tooltips for all three root buttons so the bottom bar uses the standard shared-help behavior.
- [#] configure the exact idle colors: Bright for current, Vivid for out of date, Medium when unavailable/checking.
- [#] provide J's exact shared-tooltip copy: current — `Click here for the download page, you are seemingly up to date on this build`; cannot-assess — `open the downloads page for the painter, cannot assess if you are out of date`; out-of-date — `opens the downloads page for the latest build, your current build is out of date!`.
- [#] route its click to the canonical page with a cross-platform system-browser seam; prove no download/install process is started.
- [+] include configured command-bar hotspots in the already-running tooltip frame-loop selection.

### phase-4 — release-time website publication
- [#] require release tag `vMAJOR.MINOR.PATCH` to match `thaum-painter-entrypoint`’s package version before tagging/publishing.
- [+] after GitHub Actions completes, verify the tag and all three named archives exist before touching the website worktree.
- [+] update the canonical page source’s displayed version and GitHub-generated release notes, its static manifest, and the Development entry’s single page link; run the existing website source generator.
- [+] commit/push the exact source and generated website files through the existing GitHub-auth seam.
- [+] ensure a failed website publish returns failure visibly and never claims the custom update endpoint is live.

### phase-5 — validation + repo-rule sweep + git commit
- [ ] run targeted Rust tests, format, and a built local Painter smoke test across current/outdated/unavailable responses.
- [ ] validate Linux/Windows/Mac compile paths retain the status button and browser action without platform-specific branches.
- [ ] run controlled release-script proof against the expected site files and GitHub-release verification boundary.
- [ ] run applicable encapsulation/repo-rule review and leave a clear dirty-state note if stopping before commit.
- [ ] git commit if approved.

## post-implementation-notes
- audit-2026-09-10: added the missing generated-release-notes publisher path. The guard was exercised for a mismatched package/tag before any authentication, tag, workflow, or website side effect. A real verified three-asset release, live HTTP result, and browser/manual tooltip smoke remain intentionally deferred.
- plan-finished: false
- encapsulation-git-commit: false
