# thaum-painter/domain/user-state

## purpose
Own the machine-local user session state that survives app restarts on one machine: window/camera/tool/hand preferences plus the persisted UI session. This is "not profile saves" — it is per-user convenience state for the local app, not the saved document or a portable profile.

## owns
- the `PainterUserSessionState` shape and its `schema_version`
- per-user state file naming and path resolution under the artifacts root
- load/save/round-trip semantics for machine-local user session state
- default camera resolution for persisted sessions

## does not own
- saved painter documents or their schema (`file/file-schema/`, `file/storage/`)
- where documents live on disk or autosave policy (`file/storage/`)
- live in-memory session state (`painter-session/`)
- renderer-side persisted state shapes (consumed from `thaum-renderer-domain`)

## children-encapsulations
- `user-session-state/`
  - default

## contents
- `contract.md`
  - user-state contract
- `user-session-state/user_session_state.rs`
  - rust shapes and load/save helpers for machine-local user session state

## dependencies
- `thaum-painter/domain/` (painter-session state shapes, tool-state)
- `thaum-renderer-domain` (persisted renderer UI session state shapes)

## exposed interfaces
### painter_session_state_path — resolve one user's machine-local session-state file path
send: `&Path` (artifacts root), `&str` (user id)
returns: `PathBuf`
effects: none
via: rust fn

### load_painter_user_session_state — load one machine-local user session state file
send: `&Path`
returns: `anyhow::Result<Option<PainterUserSessionState>>` (`None` when absent)
effects: read-files
via: rust fn

### save_painter_user_session_state — save one machine-local user session state file
send: `&Path`, `&PainterUserSessionState`
returns: `anyhow::Result<()>`
effects: write-files
via: rust fn

### build_user_session_state — assemble one `PainterUserSessionState` from live session pieces
send: live session state pieces
returns: `PainterUserSessionState`
effects: none
via: rust fn

## interface consumers
- `orchestration/` (app boot loads persisted session; shutdown/periodic saves)

## artifacts
- none

## tests
- none yet — shapes are exercised indirectly through `orchestration/build-commands` round-trip use

## data
- none

## notes
- renamed from `persistence/` (2026-09-07, J-approved): the name `persistence` collided with `file/storage/` also being persistence. "user-state" says what it is: machine-local prefs + session, not profile saves.
- `user-session-state/` content moved unchanged in the rename.
