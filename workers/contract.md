# /home/j/Repos/thaum-painter/workers

## purpose
Own bounded background/async seams for thaum-painter, such as future autosave scheduling.

## owns
- background/async execution seams that must not block editing-session state

## does not own
- live editing session truth (owned by `domain/painter-session/`)
- persistence policy itself (owned by `domain/file/storage/`; a worker may trigger it, not own it)
- document semantics — the session host treats records as opaque and never applies them; the `Welcome` snapshot is supplied by the owning app via a source seam

## children-encapsulations
- `session-host/`
  - default
- `session-client/`
  - default
- `session-net/`
  - default

## contents
- `session-host/`
  - default
- `session-client/`
  - default
- `session-net/`
  - default

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/storage/` (via `session-host/`: record + document serde types)

## exposed interfaces
- via `session-host/`: `SessionHost`, protocol types, `spawn_session_host_server`
- via `session-client/`: `SessionClient`, `SessionClientError`
- via `session-net/`: `SessionNet`, `SessionNetBoot`, env-boot helpers

## interface consumers
- future `orchestration/` boot flow

## artifacts
- none

## tests
- via `session-host/`: 12 inline + socket tests
- via `session-client/`: 6 socket tests (join, convergence, late-join replay, presence, denial, disconnect)
- via `session-net/`: host↔client convergence through the unified seam

## data
- none

## notes
- The old painter's hosted/shared session store deferral is now superseded: `session-host/` is the first real multiplayer transport (host-authoritative, LAN direct-IP, TCP + NDJSON, Figma-shaped order/snapshot/rejoin model). Hosted *presence stores* (durable server-side presence beyond live connections) remain deferred.
- Open permissions is a settled truth: all users get all normal file interactions; the host is variant-agnostic, so enabling structure edits is purely a `domain/file/storage/` slice (structure-edit record variants).
- `session-client/` is the connecting mirror of `session-host/`: owns the local runtime built from the `Welcome` snapshot, apply-foreign/skip-own sync, and presence/roster caches. Clients hold documents in memory only.
- `session-net/` is the app-facing seam: one `Option<SessionNet>` in the entrypoint covers hosting and joining; env-driven boot (`THAUM_SESSION_HOST` / `THAUM_SESSION_JOIN`) is live, the session-module UI replaces it later.
