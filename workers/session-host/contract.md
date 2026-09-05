# /home/j/Repos/thaum-painter/workers/session-host

## purpose
Own the host-authoritative core and LAN transport for multiplayer painting sessions: one ordered record log, one connection roster, one presence side-channel. Figma's model at LAN scale — the host defines sync order, records are opaque, joins get a fresh snapshot.

## owns
- the session wire protocol (`ClientMessage` / `HostMessage`, versioned via `SESSION_PROTOCOL_VERSION`)
- the ordered record log (arrival order is the sync order — no timestamp arbitration; the host's owning app persists it to `actions.jsonl`)
- the connection roster and duplicate/mismatched-identity denial
- the presence side-channel (cursors only in v1) — presence never enters the record log
- the `Welcome` join flow: fresh document snapshot + log-length cursor, then live records (Figma's reconnect model; rejoin = fresh snapshot, no delta catch-up)
- TCP + NDJSON transport (thread-per-connection, dedicated writer tasks, disconnect → roster broadcast, log untouched)

## does not own
- document semantics — records are opaque to the host; it never applies them. The `Welcome` snapshot is supplied by the owning app through `set_snapshot_source`
- persistence policy (`domain/file/storage/`; the host's owning app owns saves in session mode)
- the client side of the connection (`workers/session-client/`, future)
- session UI (entrypoint session module, future)
- record/structure-edit shapes themselves (`domain/file/storage/` — open perms for all users is enabled there by structure-edit record variants; the host is variant-agnostic, so open perms cost the transport nothing)

## children-encapsulations
- none

## contents
- `session_host.rs` — protocol types, `SessionHost` core (transport-agnostic, in-memory), rejections, tests
- `tcp.rs` — TCP + NDJSON listener wrapper (`spawn_session_host_server`), default port 4747 (`THAUM_SESSION_HOST_PORT` overrides), socket tests

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/storage/` (record + document types, serde only)

## exposed interfaces
- `SessionHost` — `handle_client_message`, `disconnect`, `take_outgoing`, `set_snapshot_source`, `records`, `roster`
- `ClientMessage` / `HostMessage` / `SessionUser` / `ClientRejection` / `SESSION_PROTOCOL_VERSION`
- `spawn_session_host_server`, `host_port_from_env`, `SessionHostServer::shutdown`

## interface consumers
- future `orchestration/entrypoint/` (host a session: create `SessionHost`, wire snapshot source, spawn server)
- future `workers/session-client/` (shares the protocol types)

## artifacts
- none (runtime-only)

## tests
- `session_host.rs` inline tests (9): welcome/roster join flow, arrival-order log with others-only broadcast, presence never logged, version/duplicate/not-joined/identity denials, missing snapshot source denial, disconnect semantics, ping/pong, JSON round-trip
- `tcp.rs` socket tests (2): two real clients converge through sockets (welcome, roster, record relay, presence relay, duplicate denial), disconnect shrinks the roster for survivors

## data
- none

## notes
- Source-of-truth from J (2026-09-05): open permissions — every user can do every normal file interaction (layers, property blocks, menus). The transport is variant-agnostic so this needs no host-side restriction, ever.
- Source-of-truth from J: host owns all saves in session mode for now; connector users hold the document in memory only. May be revisited later.
- Wire format: newline-delimited JSON, one `ClientMessage` per line in, one `HostMessage` per line out. A garbage line fails the connection loudly rather than guessing.
- In-process socket tests must close with `shutdown(Both)` — dropping the local handle does not deliver EOF while the server's writer clone holds the socket.
