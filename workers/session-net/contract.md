# thaum-painter/workers/session-net

## purpose
The entrypoint-facing seam over the host/client pair. The owning painter app holds one `Option<SessionNet>` and treats hosting and joining identically: `publish(record)` for records it just applied locally, `sync(&mut runtime)` to pull everyone else's in host order.

## owns
- the host/client mode switch (`SessionNet::Host` wraps core + TCP server; `SessionNet::Client` wraps the client)
- the env boot shape (`THAUM_SESSION_HOST[=port]`, `THAUM_SESSION_JOIN=ip[:port]` via `session_net_boot_from_env`; join wins if both set — one process, one side)
- the host-side consumed cursor (applies foreign records from the host core log into the app runtime; own records skipped — applied at publish)
- the identity → wire-user mapping (`session_user_from_identity`)
- default port resolution for bare `THAUM_SESSION_JOIN=ip` (4747)

## does not own
- the runtime (the app's live `SharedDocumentRuntime` stays the single source of truth `sync` applies into)
- host core / protocol / transport (`../session-host/`), client internals (`../session-client/`)
- persistence policy — clients keep the document in memory (host owns saves; settled truth from J)
- session UI (env-driven boot is the v1; the session-module UI slice replaces it)

## children-encapsulations
- none

## contents
- `session_net.rs` — `SessionNet` (host/join/publish/sync/roster/cursor/is_connected), `SessionNetBoot` + env parse, `join_address`, `session_user_from_identity`, one full host↔client convergence test

## dependencies
- `../session-host/` (core, server, `SessionUser`)
- `../session-client/` (`SessionClient`, `SessionClientError`)
- `thaum-painter/domain/file/storage/` (runtime, record, document)

## exposed interfaces
- `SessionNet` — `host`, `join`, `publish`, `sync`, `user_id`, `is_host`, `is_connected`, `roster`, `cursor`
- `SessionNetBoot`, `session_net_boot_from_env`, `join_address`, `session_user_from_identity`

## interface consumers
- `orchestration/entrypoint/` (boot env parse → `SessionNet`; frame loop: publish new `runtime.actions`, `sync`, resync mirrors on applied)

## artifacts
- none (runtime-only)

## tests
- `host_and_client_converge_through_the_unified_seam` — real sockets, both directions through the one seam shape the entrypoint uses

## notes
- Convergence model (Figma's): the host snapshot source must capture the document as of server start (the host log starts empty then); joiners rebuild via fresh snapshot + full-log replay (`Welcome` + every historical `Record`, cursor from 0). Structure edits that still bypass the record log are not replayed — closed by the `domain/file/storage/` structure-record slice.
- The entrypoint publishes by frame-end catch-up: anything that appended to `runtime.actions` since last frame goes out in order (strokes, revert records, selection) — no per-call-site wiring.
