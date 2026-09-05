# workers/session-host — multiplayer session host (LAN, host-authoritative)

## implementation-rules
- update checklist states for this plan directly while working.
- tests/ in the encapsulation for testing surfaces.
- this plan is for exactly one target encapsulation: `workers/session-host/`.
- cross-encapsulation work (entrypoint wiring, client seam, sync changes) is referenced, not absorbed.
- phase-1 is docs / contract / truth alignment; last phase is validation + repo rules + commit.

## status key
- `[ ]` not done
- `[+]` implemented
- `[#]` tested

## pre-implementation-note
The convergence seam exists (`domain/painter-session/sync/`): N sessions converge by applying one totally-ordered `SharedDocumentActionRecord` log; the loopback bus is the transport stand-in. Session identity exists (`domain/painter-session/identity/`): permanent random `user_id`, baked into every minted action id. This plan lands the first real transport in `workers/`: a host-authoritative session host for LAN direct-IP play, following Figma's shape — one host owns the order, clients mint ids offline, reconnect = fresh copy + reapply, presence rides a separate lightweight channel (never the action log).

Online-confirmed shape (2026-09-05): Figma's multiplayer tech (figma.com/blog/how-figmas-multiplayer-technology-works) uses client/server over WebSockets, one server process per document, server-defined event order (no timestamp arbitration), client-generated ids made unique by embedded client id, and reconnect-as-fresh-copy. Second-source consensus (OT-vs-CRDT guides, "you don't need a CRDT" threads): for a centralized host with structured data, a server-ordered LWW operation log beats CRDTs in simplicity — exactly our existing sync model. No CRDT needed; do not introduce one.

## target-encapsulation
- `workers/session-host/`: new

## settled design truths
- Transport: TCP with newline-delimited JSON (NDJSON). Zero new dependencies (serde + std::net); debuggable by hand; WebSocket adds a dep for no native-client benefit.
- Host authority: host assigns record order at receipt (arrival order), appends to its `actions.jsonl`, broadcasts to all clients. No timestamps in conflict logic.
- Join flow (Figma reconnect model): `Hello` → host replies `Welcome` with a full document snapshot (serialized `SharedDocumentFile` + current log length as the read cursor) → client resets its runtime from the snapshot → live `Record` broadcasts apply after the cursor.
- Reconnect: same `user_id` rejoins as the same peer with a fresh snapshot. No delta catch-up in v1.
- Presence (`cursor`, hand state, tool) rides separate `Presence` messages; never enters the action log / undo history.
- Host owns persistence in session mode; remote clients receive records only. Client-side persistence gating is an entrypoint/storage decision — cross-reference, do not absorb.
- Structure edits: **open permissions for all users** (source-of-truth from J, 2026-09-05 — no host-only restriction, no backing into a corner). The host treats records as opaque, so this costs the transport nothing. The enabling domain work is a separate slice: `SharedDocumentAction` gains structure-edit variants (LayerAdded/Removed/Renamed, PropertyBlock timing/split/merge — `split_data_propagation_record` already proves the pattern) so every normal file interaction rides the one totally-ordered log; `document.json` demotes to a host-saved materialized view. Cross-session undo of structure edits is a later slice (undo stacks are content-only today) and does not block open perms.

## protocol (v1, owned here as message types)
- Client→host: `Hello { user_id, display_name, presence_color, protocol_version }`, `Action { record }`, `Presence { cursor, hand_state }`, `Ping`.
- Host→client: `Welcome { snapshot, log_length, roster }`, `Record { record }` (broadcast, includes host's own edits), `Presence { user_id, cursor, hand_state }`, `Roster { users }`, `Denied { reason }`, `Pong`.
- Version check at `Hello`; mismatch → `Denied { reason: "protocol-version" }`.

## artifacts
- none (runtime-only; no canonical artifacts owned)

## tests
- `workers/session-host/` inline test module:
  - host orders interleaved client actions into one log and broadcasts to all
  - `Welcome` snapshot + cursor makes a late joiner converge with existing sessions (reuses the sync convergence assertions' shape)
  - duplicate user_id join is denied; unknown-version join is denied
  - presence messages never enter the record log
  - protocol round-trips through NDJSON lines

## phases
### phase-1 — docs + contract + truth alignment
- [ ] write `workers/session-host/contract.md` (owns: protocol types, connection roster, ordered log, broadcast; does not own: document semantics, file storage, entrypoint UI)
- [ ] update `workers/contract.md` child list; keep the hosted-presence-store deferral note accurate
- [ ] record the open-permissions truth: all users get all normal file interactions; host is variant-agnostic; structure edits become records via a follow-on domain slice
- [ ] cross-reference the client-side persistence-gating question for the entrypoint/storage owners

### phase-2 — protocol + host core (in-memory)
- [ ] protocol message types with serde, versioned
- [ ] `SessionHost` core: roster, ordered record log, broadcast fan-out, snapshot builder for `Welcome`
- [ ] in-memory tests including convergence parity with the loopback bus shape

### phase-3 — TCP transport
- [ ] std::net TCP listener, thread-per-client, NDJSON frame encode/decode
- [ ] disconnect handling: roster update, no record loss (host log remains truth)
- [ ] direct-IP listen on a fixed default port; bind address configurable via env

### phase-4 — validation + repo rules + commit
- [ ] full workspace test run green
- [ ] smoke: one host + two in-process clients converge through real sockets
- [ ] contract/plan checklists closed; git commit

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false

## follow-on plans (not this encapsulation)
- `domain/file/storage/` structure-edit record variants — open-perms prerequisite: layers/property blocks become apply-locally-then-publish records; structure undo (revert records) follows
- `workers/session-client/` — connect/send/receive seam the entrypoint frame loop drains (same apply-in-order shape as `sync_from_bus`))
- entrypoint session module (host/join/status UI in the module bottom bar)
- client persistence gating (host owns saves in session mode)
- presence rendering (remote cursors as scene quads)
