# project plan — relay transport (multiplayer v2: invite-code sessions beyond LAN)

## pre-implementation-note
Multiplayer v1 is real and battle-tested on LAN: host-authoritative core (`workers/session-host/`, transport-agnostic by design), TCP/NDJSON transport (`workers/session-host/tcp.rs`), client with auto-rejoin (`workers/session-client/`), one `Option<SessionNet>` seam in the entrypoint (`workers/session-net/`), session panel UI (`domain/modules/individuals/session-panel/`), and re-seed-on-swap (`SessionNet::reseed_host`). Settled truth this plan builds on: records are the only wire content (content + structure + revert records), rejoin = fresh snapshot + full replay, host owns saves in session mode, client-side disk policy is gated by `persist_to_disk`.

The gap: sessions only work when both machines can reach each other's IP — same-LAN in practice. Consumers need "paste a code, paint together". This plan adds a **relay lane**: a standalone relay server that both host and joiners dial OUT to (no port forwarding, no VPN, works from any network), routed by invite code. It is not a fork of the netcode — the same record protocol and the same host-authoritative core flow through a second transport.

Deployment truth (J 2026-09-08): JOBO (always-on Linux server box, 32c/122GB, headless, no screen) hosts the dev-stage relay on a public port. The relay is a standalone binary, deliberately provider-agnostic — the same binary moves to a paid VPS/service later; clients only ever hold a relay URL. LAN direct stays the default lane and stays free forever. Self-host = hand a friend the relay binary.

## ownership rules
- The host core (`SessionHost`) never learns about the relay. It stays transport-agnostic; the relay client wraps it exactly like `tcp.rs` does today.
- `SessionNet` stays the only session seam the entrypoint touches. A relay session is just a `SessionNet` whose transport is the relay lane. The frame loop does not change.
- The relay server never interprets records. It routes opaque lines per room, like the host core treats records: order + route + caps, nothing else. Document semantics stay in `domain/`.
- The session panel renders invite truth verbatim from `SessionNet` (codes when relayed, `ip:port` when LAN). No derivation in UI.
- Security is not a later slice: TLS + token auth + record-size caps + room/rate caps land with the relay's first build, because the JOBO dev relay is internet-exposed from day one.
- No platform-specific code. `std::net` TCP + `rustls`/WebSocket over it run on all three lanes; no conditional compilation.

## affected-encapsulations
- `workers/session-relay/`: new — relay protocol, relay server binary, relay client lane
- `workers/session-net/`: edit — relay lane joins `host()`/`join()` as a third session shape
- `domain/modules/individuals/session-panel/`: edit — invite-code field + relay invite display
- `orchestration/build-commands/`: edit — env boot for relay, panel-action wiring, invite copy format
- `orchestration/` deploy notes + script: new — JOBO systemd unit + TLS/TOFU setup for the dev relay

## encapsulation-plans
- `workers/session-relay/`: `plans/home-j-repos-thaum-painter-workers-session-relay-plan.md`
- `workers/session-net/`: `plans/home-j-repos-thaum-painter-workers-session-net-plan.md`
- `domain/modules/individuals/session-panel/`: `plans/home-j-repos-thaum-painter-domain-modules-individuals-session-panel-plan.md`
- `orchestration/build-commands/`: `plans/home-j-repos-thaum-painter-orchestration-build-commands-plan.md`

## encapsulation-details
### `workers/session-relay/`
- action: new
- intent: Own the relay transport end to end: the wire protocol, the room-routing server binary, and the client-side lane both session ends use. One boundary owns everything swappable about "how two machines find each other over the internet".
- interface delta: `relay-server` bin (`thaum-session-relay`): `--port`, `--tls-cert/--tls-key` (or TOFU self-signed gen), room registry with caps. Client lane: `RelayLink::connect(url, room_code, token, role)` — the same `send_line/recv_line` shape `tcp.rs` wraps, so the host core consumes it unchanged.
- dependencies: `workers/session-host/` — reused unchanged behind the lane (host role); `workers/session-client/` — reused unchanged behind the lane (joiner role); `rustls` + a minimal WS or raw-TLS line protocol (decide in the encapsulation plan; raw TLS + length-prefixed NDJSON is the lean default over full WebSocket).
- consumers: `workers/session-net/` (relay session shape), orchestration boot (relay URL config).
- artifacts: none operational; `data/relay-shape-notes.md` allowed for protocol analysis.
- tests: `tests/` in-encapsulation — room join/deny (bad token), record passthrough order per room, size-cap enforcement, room-cap enforcement, host-reseed still reaches relay clients, reconnect after relay drop.
- data: `data/relay-shape-notes.md` (non-operational)

### `workers/session-net/`
- action: edit
- intent: A relay session must be indistinguishable from a LAN session to the entrypoint: same publish/sync/rejoin/reseed semantics, different dial target.
- interface delta: `SessionNet::host_relay(url, code, token, user, seed)` + `SessionNet::join_relay(url, code, token, user)`; `SessionNetBoot::Relay` variant; invite truth returns the code for relay sessions.
- dependencies: `workers/session-relay/` — the client lane; `workers/session-host/`, `workers/session-client/` — unchanged reuse.
- consumers: orchestration frame loop (unchanged calls), session panel (invite display).
- artifacts: none
- tests: relay-mode rejoin convergence, reseed over relay, take_rejoined republish guard in relay mode.
- data: none

### `domain/modules/individuals/session-panel/`
- action: edit
- intent: The JOIN field accepts either an `ip:port` (LAN lane, unchanged) or an invite code (relay lane); the panel shows the code + copy button when hosting over relay.
- interface delta: none new — `JoinRequested{addr}` carries either shape; routing by shape happens in orchestration, the panel stays a dumb view.
- dependencies: `workers/session-net/` invite truth only.
- consumers: orchestration applies the same actions as today.
- artifacts: none
- tests: code-shaped input passthrough, relay invite display from synced truth.
- data: none

### `orchestration/build-commands/`
- action: edit
- intent: Boot and wire the relay lane: env vars (`THAUM_SESSION_RELAY`, `THAUM_SESSION_CODE`, `THAUM_SESSION_TOKEN`), panel-action routing (code vs direct address), and invite-copy formatting.
- interface delta: none beyond env boot variants + action application.
- dependencies: `workers/session-net/` — relay lane.
- consumers: the entrypoint frame loop (no changes to publish/sync/persist gates).
- artifacts: none
- tests: boot variant parsing, code-vs-address routing.
- data: none

### `orchestration/` relay deployment (notes + script)
- action: new (deploy notes + small script inside the orchestration boundary; not a new encapsulation)
- intent: One-command JOBO deployment: build relay bin, systemd unit (restart-on-fail), TLS cert bootstrap, firewall note for the chosen port.
- interface delta: none
- dependencies: `workers/session-relay/` server bin.
- consumers: J operating the dev relay.
- artifacts: `orchestration/session-relay/deploy/` — unit file + runbook.
- tests: none (manual bring-up checklist)
- data: none

## open questions (settled 2026-09-08)
- [x] TLS strategy: real cert via a subdomain of jartanddesign.com pointed at JOBO (Let's Encrypt, certbot outside the binary; the relay takes cert/key paths). Ops step in the deploy phase: DNS A record + port-forward 443 (or the chosen port) + certbot renewal.
- [x] Transport inside TLS: raw TLS + length-prefixed NDJSON (mirrors the current wire; no WebSocket).
- [x] Invite code shape: ONE short field, no prefix: `<room6>-<token10>` (17 chars, ambiguous-glyph-free alphabet). The JOIN field routes by shape: contains `:` = ip:port (LAN), otherwise = relay code.
- [x] Relay persistence: NONE. Rooms live only while connected; zero disk state. The host owns all document truth (settled model) — the relay only routes.

## phases
### phase-1 — architecture alignment + encapsulation ordering
- [x] touched encapsulations identified (session-relay new; session-net, session-panel, orchestration edit)
- [x] actions recorded per encapsulation
- [x] interface deltas recorded
- [x] dependencies + consumer expectations recorded
- [x] every touched encapsulation gets its own plan at phase start
- [x] order: relay (protocol+server+lane) → session-net lane → orchestration wiring → session-panel UX → JOBO deploy → verification

### phase-2 — plan + build `workers/session-relay/`
- [x] write `plans/home-j-repos-thaum-painter-workers-session-relay-plan.md`
- [x] resolve open questions (TLS shape, transport-in-TLS, code shape, persistence)
- [x] relay protocol: room join with token auth, record passthrough in arrival order, presence passthrough, deny reasons, close/room-end
- [x] relay server bin: room registry, per-room client sets, record-size cap, room-cap, rate guard, TLS
- [x] relay client lane: `RelayLink` with the same line-send/recv shape as `tcp.rs`; host-role and joiner-role wrappers around the existing cores
- [x] tests: join/deny, passthrough order, caps, drop/reconnect
- [x] TLS slice (J 2026-09-08): rustls both ends — `TlsRelayServerAcceptor` (cert/key PEM, certbot output) wires the bin's real listener; client dials TLS via webpki roots, plaintext only ever loopback; handshake-appdata collision captured + replayed; in-process TLS end-to-end test green

### phase-3 — plan + edit `workers/session-net/`
- [x] write `plans/home-j-repos-thaum-painter-workers-session-net-plan.md`
- [x] `SessionNet::Relay` shape wiring host/joiner cores through the lane
- [x] rejoin over relay (existing backoff path), reseed over relay, invite truth returns the code
- [x] tests: relay-mode convergence, rejoin, reseed

### phase-4 — plan + edit `orchestration/build-commands/`
- [x] write `plans/home-j-repos-thaum-painter-orchestration-build-commands-plan.md`
- [x] env boot variants for relay
- [x] panel-action routing: code → relay lane, `ip:port` → direct lane
- [x] invite-copy format for relay sessions
- [x] tests: boot parsing, routing

### phase-5 — plan + edit `domain/modules/individuals/session-panel/`
- [x] write `plans/home-j-repos-thaum-painter-domain-modules-individuals-session-panel-plan.md`
- [x] code-shaped input accepted in the join field (hint text names both shapes)
- [x] relay invite display + copy from synced truth
- [x] tests: passthrough + display

### phase-6 — JOBO deployment
- [x] deploy notes + script under `orchestration/session-relay/deploy/`
- [x] systemd unit, TLS bootstrap, firewall/port checklist
- [ ] live bring-up on JOBO + external-network smoke test (join from artinator over the relay) — blocked on the DNS A record + certbot cert (runbook step "One-time: DNS + cert")

### phase-7 — final verification + repo-rule sweep + git commit
- [ ] every touched encapsulation has a linked, satisfied encapsulation plan
- [ ] detail blocks complete (action/intent/interface/dependencies/consumers/artifacts/tests/data)
- [ ] full test suite green; relay end-to-end matrix (LAN direct, relay same-LAN, relay cross-network via VPN-disabled path)
- [ ] encapsulation-rule review across touched encapsulations
- [ ] git commit if approved

## deliberately not in this project
- Accounts/user database (invite tokens are per-room, not per-identity)
- TURN/ICE/P2P hole-punching (relay is the chosen path; direct LAN stays for the common case)
- Relay-side persistence or history (rooms are ephemeral; the host owns saves)
- Paid-service integration (the swap point is the relay URL + the same binary elsewhere)
- Chat/voice (presence already ships; nothing new)

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
