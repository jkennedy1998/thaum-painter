# project-plan: thaum-painter file-boundary cleanup + load-time schema gate

## implementation-rules
- update checklist states for this plan directly while working.
- use `tests/` inside the touched encapsulation for testing.
- canonical plan location: `plans/` in the thaum-painter repo.
- this is deliberately ONE combined plan (user request, 2026-09-07 operator-2 chat); encapsulation-local plans are folded into the phases below instead of linked files. If any phase grows past ~10 tasks, split it out into its own encapsulation plan file at that point.
- no behavior changes beyond the schema gate; everything else is naming/moving/deleting.
- the dropped idea is recorded on purpose: extracting an atomic-write helper into `tools/` was evaluated and REJECTED (web/OPFS backend would not consume it; no second consumer). Do not revisit without a second real consumer.

## status key
- `[ ]` not done
- `[+]` implemented
- `[#]` tested

## pre-implementation-note
Written 2026-09-07 from the operator-2 chat. The file surface was mapped this morning: `domain/file/` holds `manifest/`, `storage/`, `import-export/`, plus a stray top-level `document_locations.rs`; `domain/persistence/` holds `user-session-state/`. Separately, operator-1's bars/binary work (see `context/bars-binary-design-truth.md`) will break the saved file schema (v1 -> v2, no importer, no migration), so old files must be rejected cleanly at load time instead of failing obscurely. Operator-2 owns this plan and the schema-gate task; operator-1 stays on bars/animation and was told not to pick this up. Current code facts verified: `load_document_file` in `storage.rs` deserializes `SharedDocumentFile` directly and never checks `file_kind`/`schema_version` (constants `SHARED_DOCUMENT_SCHEMA_VERSION = 1`); `manifest.rs` has a kind+version gate in `parse_manifest` but it is not wired into the load path; `migrate_legacy_painter_file_root` lives in `document_locations.rs` and both old and new roots are empty on this machine.

## decided truths (user-confirmed 2026-09-07, operator-2 chat)
- `manifest/` renames to `file-schema/` — "what's the current schema version" is the sentence the code should say.
- `storage/` stays inside `domain/file/` — it owns opinions (platform roots, snapshot vs ticket log, atomic-write policy), and tools have no opinions.
- NO extraction of the atomic-write helper into `tools/` — web backend would not use it; dropped until a second consumer exists.
- `import-export/` stays as the copies-for-humans room (currently contract-only).
- `persistence/` renames to `user-state/` — machine-local prefs + session ("not profile saves"); the name `persistence` collides with storage also being persistence.
- `document_locations.rs` root resolution moves under `storage/` — "where files go" is storage policy.
- delete `migrate_legacy_painter_file_root` — legacy roots are empty; user fixes friend files case-by-case; no long-term migration support.
- schema break for bars work: no importer, no migration; old files get clean "unsupported" recognition, never a crash.
- operator-2 owns the load-time schema rejection; operator-1 is on animation pieces of layers and will not touch it.

## affected-encapsulations
- `thaum-painter/domain/file/`: edit
- `thaum-painter/domain/file/manifest/`: edit (rename to `file-schema/`)
- `thaum-painter/domain/file/storage/`: edit (absorb root resolution, wire schema gate)
- `thaum-painter/domain/persistence/`: delete (renamed to `user-state/`)
- `thaum-painter/domain/user-state/`: new

## phases
### phase-1 — architecture alignment + ordering
- [+] confirm touched encapsulation list and actions above with J (renames only, no behavior change except the gate)
- [+] confirm code-naming deltas: `manifest.rs` -> `file_schema.rs`, `MANIFEST_VERSION` -> `FILE_SCHEMA_VERSION`, `parse_manifest` -> `parse_file_schema` (J approved 2026-09-07)
- [ ] record the decided truths into each touched contract's notes
- [+] order work: rename file-schema -> storage moves/gate -> user-state rename -> verify (phases 2-3 executed in this order)

### phase-2 — `domain/file/manifest/` -> `domain/file/file-schema/` (rename, docs-first)
- [+] update `domain/file/contract.md` children list + purpose wording (manifest -> file-schema)
- [+] rewrite `manifest/contract.md` as `file-schema/contract.md`: owns file kind/version markers + schema-version gate truth
- [+] rename `manifest.rs` -> `file_schema.rs`; constants/types per phase-1 decision (`FILE_SCHEMA_KIND`, `FILE_SCHEMA_VERSION`, `FileSchema`, `parse_file_schema`)
- [+] move example + schema JSON artifacts under `file-schema/` (version markers kept: v1 is a real generation marker)
- [+] update all `use` paths / module references across the repo (lib.rs, timeline-state, render-space, pieces, interpolation, properties)
- [+] run existing file-schema tests; all green (343 passed, workspace check clean)

### phase-3 — `domain/file/storage/` (root moves + load-time schema gate)
- [+] move `document_locations.rs` root-resolution functions under `storage/`; update `domain/file/contract.md` contents list (done: file moved via git mv, lib.rs path updated)
- [+] delete `migrate_legacy_painter_file_root` + `legacy_painter_file_root` and their call site (roots empty; ~10-line deletion)
- [+] add schema gate in the load seam: `load_document_file` checks `file_kind` + `schema_version` BEFORE deserializing the body (`ensure_supported_file_schema` on the decoded JSON value)
- [+] gate returns a typed `UnsupportedFileError` (`UnsupportedFileReason::KindMismatch | VersionMismatch`, file path, found vs supported) distinct from generic parse errors; user-facing message is EXPLICIT about the version difference ("file is schema v1, this app reads v2")
- [#] trace the open-file caller path and map `UnsupportedFile` to a clean user-facing rejection: verified in `orchestration/build-commands/src/main.rs` `file:open` — `UnsupportedFileError` downcasts to a soft rejection (file does not open, no partial state, session keeps running); every other failure still propagates
- [#] tests green: v-next file rejects with version message; wrong `file_kind` rejects; missing kind/version rejects as unsupported; valid v1 file loads unchanged — verified 2026-09-07 after operator-1's bars refactor finished (the earlier blocker cleared); `cargo test -p thaum-painter-domain` = 336 + 2 passed, workspace check clean

### phase-4 — `domain/persistence/` -> `domain/user-state/` (rename)
- [+] rename `domain/persistence/` -> `domain/user-state/`; `user-session-state/` content moves unchanged (git mv 2026-09-07)
- [+] `user-state/contract.md` written: machine-local prefs + session, "not profile saves" (no contract existed before — created)
- [+] update all `use` paths, `domain/lib.rs` exports, and `domain/contract.md` children list (lib.rs `#[path]` updated; `domain/contract.md` now lists `user-state/` which was previously unlisted; `painter-tools/contract.md` consumer ref updated)
- [#] run user-session-state tests; all green (no dedicated inline tests exist — shapes verified via full workspace run: 376 passed)

### phase-5 — final verification + git commit
- [#] full `cargo test` across the workspace; all green (376 passed, 0 failed, 2026-09-07)
- [#] verify no remaining references to `manifest`, `persistence/`, or `migrate_legacy` anywhere (grep sweep; stale contract mentions in rendering/camera, render-space, rendering, painter-document, painter-document/timing, painter-session/tool-state, painter-session/selection, layers-panel, file-schema example md all fixed; remaining hits are rename-history notes only)
- [#] verify contracts match reality for `domain/file/`, `file-schema/`, `storage/`, `user-state/` (paths, contents, dependencies, consumers)
- [#] confirm the dropped tools/atomic-write decision is recorded in this plan + storage contract notes (storage notes now carry gate, rename, and dropped-idea entries)
- [#] git commit (J approved continuation 2026-09-07)

## J's answers (2026-09-07, folded in from open questions)
1. Naming approved: `file_schema.rs` / `FILE_SCHEMA_VERSION` / `parse_file_schema`.
2. Rejection message is EXPLICIT: name the found schema version vs the supported one (e.g. "file is schema v1, this app reads v2"). Supporting truth: the schema version only changes when the shape breaks, so the version number alone carries enough meaning to describe the mismatch — and recording per-version shapes later makes future migrational content easy to store.
3. Alignment with the bars plan settled: the version BUMP stays in `plans/project-thaum-painter-binary-bars-plan.md` (it breaks the shape, it bumps the number); this plan owns the load-time rejection MECHANISM (typed error + clean open-flow rejection). Cross-reference note added to the bars plan.

## alignment with operator-1's binary-bars plan
- bars plan already declares: "Schema version bump — no migration pass, no importer; old files recognized as unsupported without crashing (mechanism minimal, deferred detail)."
- this plan is where that deferred mechanism lands (phase-3). Bars plan tasks: bump `SHARED_DOCUMENT_SCHEMA_VERSION` (or renamed `FILE_SCHEMA_VERSION`) and keep old-schema files failing into the typed rejection instead of a serde error.
- ordering: this plan's phase-3 (gate) should land before or with the bars bump so v1 files reject cleanly the moment v2 exists.
