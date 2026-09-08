# thaum-painter/domain/modules/individuals/session-panel

## purpose
Own the one user surface for LAN multiplayer: an always-attached session chip whose label IS the session status, plus a click-open detail panel (roster, invite copy, join field, name edit, log-lite events).

## owns
- the chip: status dot + label (`OFFLINE` / `HOSTING ip:port` / `CONNECTED (n)` / `RECONNECTING...` / `ENDED`), derived only from synced fields; click toggles the panel
- the panel's offline flow: one-click host, one-field join (`ip:port` + Enter), name edit
- the panel's in-session flow: roster rows (host crowned at entry zero, you marked, presence colors), `[COPY INVITE]` (host), `[LEAVE]`, name edit
- the log-lite event lines (newest three, verbatim from the orchestration layer)
- `SessionPanelAction` emission (`HostRequested`, `JoinRequested`, `CopyInvite`, `LeaveRequested`, `SetDisplayName`) — at most one per frame via `SessionPanelState::take_pending_action`
- focused-field text entry through the registry's key-capture seam (`on_key_capture`, the controls-panel rebind pattern)

## does not own
- any `SessionNet` / `SessionHost` / `SessionClient` / socket truth — the module renders only what `SessionPanelState::sync` was told and never invents state (no optimistic connected, no filler)
- invite-address computation (`SessionNet::invite_addresses()` is the single source; the panel renders it verbatim)
- clipboard access (orchestration applies `CopyInvite` through arboard and reports back via an event line)
- session boot/leave semantics (orchestration owns the one `Option<SessionNet>`)
- the roster wire truth (`workers/session-net/` contract: roster entry zero is the host)

## children-encapsulations
- none

## contents
- `contract.md`
  - session-panel contract
- `session_panel_module.rs`
  - `SessionChipModule`, `SessionPanelModule`, `SessionPanelState`, `SessionRosterRow`, `SessionPanelAction` — the chip and panel view modules over the one shared `Rc<RefCell<SessionPanelState>>`; the caller syncs truth each frame, applies the pending action onto `SessionNet`, and mirrors the panel-open flag into the registry's `set_hidden`

## dependencies
- `thaum-renderer/domain/modules/`
- `thaum-renderer/domain/modules/shared/panel-chrome/`

## exposed interfaces
### chip + session detail panel
send: session truth via `SessionPanelState::sync` fields (`in_session`, `is_host`, `connected`, `reconnecting`, `session_ended`, `peer_count`, `invite_addresses`, `roster`, `self_display_name`, `events`), plus pointer events routed by the module registry and raw key labels through key capture while a field is focused
returns: at most one pending `SessionPanelAction` per frame, taken via `SessionPanelState::take_pending_action`; the chip's click toggles `panel_open`, which the caller applies to the registry
effects: none directly — the orchestration layer applies actions to the real `SessionNet` and reflects results back through `sync`
via: `SessionChipModule`, `SessionPanelModule`, `SessionPanelState`

## interface consumers
- `thaum-painter/orchestration/build-commands/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `session_panel_module.rs`
  - light
  - validates chip label derivation from synced truth only, chip click toggle, closed-panel draw/click silence, one-action-per-frame queueing, join-field key capture (chars, Enter commit, Escape blur, fall-through when unfocused), name commit only-when-changed, leave/copy routing by side, roster crown/(you)/presence-color rendering, event passthrough verbatim with newest-lowest order, and the three-line event cap

## data
- none

## notes
- Display-name rename rides the `ClientMessage::Rename` wire variant (host updates roster truth and broadcasts the roster to every client, renamer included); the host's own rename updates the host identity and broadcasts the same way. No presence-store machinery.
- The roster contract — entry zero is always the host — lives in `SessionNet::roster` (`workers/session-net/`); the panel crowns entry zero and marks the row matching the local user id as `(you)`.
- A generic Module-trait focus/IME text seam still does not exist (same deferred cross-repo work as the layers-panel rename). This panel leans on the key-capture seam; joining therefore needs raw key labels including `.` and `:` — provided by the entrypoint's key-label map.
- Deliberately not in v1: session browser, accounts/auth, kick/ban, chat, permissions UI, host migration, durable presence stores.
