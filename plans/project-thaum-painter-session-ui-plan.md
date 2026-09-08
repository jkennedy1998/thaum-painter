# project plan — session UI (multiplayer v1 surface)

## pre-implementation-note
The multiplayer transport is already real: `workers/session-host/` (host-authoritative core + TCP/NDJSON), `workers/session-client/` (join/apply/roster mirror), `workers/session-net/` (one `Option<SessionNet>` in the entrypoint, frame-end publish + host-order sync). What is missing is the user surface: hosting/joining today only works through env vars (`THAUM_SESSION_HOST` / `THAUM_SESSION_JOIN`).

Ownership rule for this pass: the session panel is a **view + intent** module. It never touches `SessionHost`/`SessionClient`/sockets — it emits intents (`SessionPanelAction`) that the orchestration layer applies onto the one `Option<SessionNet>`. Session truth stays in `workers/`; panel truth is only what it was last told via `SessionPanelState::sync`. This is the swap-later guarantee: replace the transport, the panel doesn't know; replace the panel, the transport doesn't know.

Cross-platform rule (Windows/Linux/Mac): zero platform-specific code in this pass. Everything rides existing cross-platform seams — `std::net` TCP (already shipped), the renderer `Module` trait (winit/wgpu, already shipped), `if-addrs` (tiny crate: local interface enumeration for the invite addresses, all three OSes), `arboard` (clipboard copy of the invite, all three OSes). No lane-specific behavior, no conditional compilation.

No-fallback-content rule: every state the panel can display is real state — a denied join shows the host's actual rejection reason, a disconnected chip says disconnected. The panel never invents plausible-looking filler. If `SessionNet` says `is_connected == false`, the chip says so; there is no "optimistic connected" mode.

## current-state
- Boot: `session_net_boot_from_env` → `Option<SessionNet>`; `None` = single-user. Env boot stays (it is a real boot path, not a fallback content path) but is no longer the only way in.
- Frame loop: frame-end catch-up publish + `sync` + mirror resync already wired. Chip reads the same `SessionNet`.
- `SessionUser` already carries `display_name` + `presence_color`; the roster wire already ships names.
- No local-address enumeration anywhere; no clipboard seam in the app.

## target-encapsulation
- `thaum-painter/domain/modules/individuals/session-panel/` (new, mirrors layers-panel shape): `SessionPanelModule` + `SessionPanelState` + `SessionPanelAction`. One always-attached chip + one click-open panel.
- `workers/session-net/` gains the invite-address truth: `SessionNet::invite_addresses()` → the addresses a joiner should type. Single source of truth for "how do people reach this session" — the panel renders it verbatim, nothing else computes it.
- `workers/session-client/` gains auto-rejoin with backoff (client-owned; the entrypoint's frame loop just calls `sync` as it already does).
- Host-session-over: one explicit `HostMessage::Ended` (protocol bump) so clients render "the host ended the session" instead of a silent drop.

## core design
1. **The chip is the status.** One small always-visible module row (docked near the command bar): state dot + short label — `OFFLINE` / `HOSTING ip:port` / `CONNECTED (n)` / `RECONNECTING…`. Click opens the panel. The chip renders only what `SessionPanelState` was synced; no derivation, no guessing.
2. **Host flow = one click.** "Host Session" on the current document: `HostRequested` → orchestration builds `SessionNet::host`, spawns the server, chip flips to `HOSTING`. Painting never pauses. The invite (every reachable local `ip:port` from `invite_addresses()`) shows in the panel with one [copy] button (arboard copies `ip:port` lines). This is Minecraft open-to-LAN: the host is just the app running.
3. **Join flow = one field.** `JoinRequested { addr }` from a single text input (module keyboard hook via the existing text-entry seam patterns), [Connect] button. States rendered honestly: `CONNECTING` → `CONNECTED (n)`; denial reasons surface verbatim (version mismatch, duplicate identity, refused).
4. **Roster, lean.** Name rows in join order: presence dot (Figma filled/dotted), crown for host, "(you)" marker, editable display name for self (rides the existing `SessionUser.display_name` — rename republishes presence, no new wire type). No avatars, no profiles, no admin actions in v1.
5. **Leave is explicit and honest.** [Leave Session] on either side. Host leaving = session over: `Ended` broadcast, clients show "host ended the session" and drop to `OFFLINE` — no silent death, no host migration (explicitly deferred).
6. **Auto-rejoin is client-owned.** On unexpected drop, `session-client` retries with capped backoff; chip reads `RECONNECTING…`. Rejoin is the existing cheap path (fresh snapshot + full replay), so no delta machinery.
7. **Identity.** `display_name` defaults to the machine username (already how `session_user_from_identity` works); first rename persists in the per-user UI session state (`PersistedPainterUiSessionState`), not the document.

## deliberately not in v1
Session browser/lobby, accounts/auth (LAN trust + open perms are settled truth), kick/ban, chat (presence cursors exist), permissions UI, host migration, durable presence stores, relays/TURN (LAN direct-IP is the settled v1 transport).

## phases
### phase-1 — seam truths (workers, no UI)
- [x] `SessionNet::invite_addresses()` via `if-addrs`: every non-loopback v4 address + the port; loopback listed last as `127.0.0.1:port`. Empty list renders as "no reachable address — check your network" (real state, not filler).
- [x] `HostMessage::Ended` + protocol version bump; host API `end_session()`; client marks ended vs dropped distinctly.
- [x] `session-client` auto-rejoin: capped exponential backoff, rejoin through the existing join path; exposure for tests. Rejoin rebuilds the caller's runtime from the fresh snapshot inside `sync`; `take_rejoined()` guards republish.
- [x] socket tests: ended broadcast, rejoin after drop converges again.
- [x] roster truth: host identity is roster entry zero (`set_host_user`); `ClientMessage::Rename` + host-side rename broadcast the roster to every client.

### phase-2 — session panel module
- [x] `domain/modules/individuals/session-panel/`: chip row (offline/hosting/connected/reconnecting states + click target), panel (roster list, self-name edit, invite list + [copy], join field + [connect] when offline, [leave] when in session), `SessionPanelAction` set (`HostRequested`, `JoinRequested{addr}`, `CopyInvite`, `LeaveRequested`, `SetDisplayName`), contract.md.
- [x] module tests: state-sync rendering per chip state, one-pending-action-per-frame, denial text passthrough.

### phase-3 — orchestration wiring
- [x] Frame loop applies `SessionPanelAction` onto `SessionNet` (host/join/leave/copy/rename); `SessionPanelState::sync` fed from the same `SessionNet` each frame — one truth, one writer.
- [x] Clipboard via `arboard` behind the module's `CopyInvite` action result (success/failure surfaces in the panel's event line).
- [x] Env boot unchanged; the panel and env boot feed the same `Option<SessionNet>`, so `THAUM_SESSION_JOIN` still works for headless/CI use. Publish loop hardened to own records only (foreign records arrive via sync — the old path double-logged them on the host).
- [ ] End-to-end manual matrix: linux↔linux, linux↔windows (cross-compile lane), mac via CI lane — same LAN, two instances, paint/sync/rejoin/leave.

## tests
- workers: ended-broadcast, rejoin convergence, invite-address enumeration (loopback-only environment renders the honest empty state)
- module: chip state rendering from sync, action emission, denial passthrough, roster order/colors from the real roster
- orchestration: action→SessionNet application, no panel action when offline beyond host/join, rename persistence shape

## data
- none new (display name persists in the existing per-user UI session state)

## notes
- The chip is a module, not HUD special-casing — it lives in the module registry like every other panel, so layout/reset-layout keeps one truth.
- `arboard` + `if-addrs` are the only new dependencies; both are cross-platform and tiny. Everything else reuses shipped seams.
- Swappability: transport swap = `workers/session-*` internals + `SessionNet` API only; UI swap = the panel module only; the contract between them is `SessionPanelState::sync` + `SessionPanelAction` and nothing else.
