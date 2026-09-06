# thaum-painter/workers/session-client

## purpose
The connecting side of a hosted multiplayer session: mirror `domain/painter-session/sync/`'s `SyncedDocumentSession` semantics over the host's wire. Owns the local document runtime built from the host's `Welcome` snapshot, the read cursor, presence/roster caches, and the publish flows.

## owns
- the read cursor into the host log (`consumed_count`; starts at 0 — joiners replay the full log onto the snapshot)
- apply-foreign / skip-own sync semantics (same rule as `sync_from_bus`: foreign records apply in host order into the CALLER's runtime, own records were applied locally at publish)
- the handshake's buffered reader handoff (the reader thread inherits the `BufReader` so lines buffered past `Welcome` — replayed history — are not lost)
- presence and roster caches the owning app reads after `sync`
- reconnect policy: a fresh `connect` with the same `user_id` — fresh snapshot + full-log replay, no delta catch-up (Figma's model)
- the `Welcome`-then-history join flow consumption: the handshake consumes only `Welcome`; replayed records arrive through the live queue

## does not own
- the wire protocol or host state (`../session-host/` — the client imports its types, it is the other end)
- record/structure-edit shapes (`domain/file/storage/`)
- persistence (clients hold the document in memory only; the host's owning app owns saves — settled truth from J, 2026-09-05)
- session UI and frame-loop wiring (entrypoint, next slice)

## children-encapsulations
- none

## contents
- `session_client.rs` — `SessionClient` (`connect` returning the snapshot, `sync(&mut runtime)`, `send_action` for already-applied records, `send_presence`, caches, `shutdown`), `SessionClientError`, synchronous `Hello` handshake + reader/writer threads

## dependencies
- `thaum-painter/domain/file/storage/` (runtime, record, document types)
- `../session-host/` (protocol types: `ClientMessage`, `HostMessage`, `SessionUser`, `SESSION_PROTOCOL_VERSION`)

## exposed interfaces
- `SessionClient` — `connect`, `connect_with_runtime`, `sync`, `publish_action`, `publish_history_record`, `send_presence`, `ping`, `roster`, `cursor`, `consumed_count`, `is_connected`, `shutdown`
- `SessionClientError` — `Io` / `Denied` / `Timeout` / `Json` / `Disconnected`

## interface consumers
- future `orchestration/entrypoint/` (join a hosted session from the session module UI; drive `sync` from the frame loop)

## artifacts
- none (runtime-only)

## tests
- 6 inline socket tests against a real `spawn_session_host_server`: join snapshot + self-roster, two-client publish/sync convergence, late-join full-log replay, presence round-trip, duplicate identity denied, disconnect visible to the frame loop

## notes
- The frame loop drives `sync()` once per frame; wire hops are async, so callers must treat "0 applied" as "not yet", not "nothing exists" (test helper `sync_for` shows the bounded-retry pattern).
- Cursor truth: a client only ever consumes *foreign* records live — the host excludes the sender from its own record's broadcast — so a live `consumed_count` trails the host log by the client's own record count. On join, the full replay (including own old records) is applied from cursor 0.
- The client does NOT own the runtime: `sync(&mut runtime)` applies into the caller's live runtime, so the entrypoint's canvas/selection mirrors resync from one object.
- Handshake is synchronous with a 5s read timeout, then threads take over. A garbage wire message loses the connection loudly (reader sets `connected=false`) rather than guessing.
