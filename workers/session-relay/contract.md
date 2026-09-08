# thaum-painter/workers/session-relay

## purpose
Own the relay transport for multiplayer painting sessions beyond LAN: the relay wire protocol, the room-routing server binary, and the client-side lane both session ends dial out through. Invite-code sessions: host and joiners all dial OUT to the relay — no port forwarding, no VPN, works from any network.

## owns
- the relay wire protocol (role-claim hello, routed frames, error/closure frames; length-prefixed NDJSON with a hard frame cap)
- invite codes: `<room6>-<token10>` minting and parsing (Crockford-style alphabet, no ambiguous glyphs); the code IS the room auth
- the room registry: one host + capped joiners per room, in-memory only, rooms die with the host connection (zero disk state — the host owns all document truth)
- routing rules: joiner lines go to the room host; host frames go to one named joiner or all; session content lines are opaque
- caps enforced at the door and per frame: frame bytes, members per room, rooms per server, per-IP join rate
- the standalone relay server binary (`thaum-session-relay`): plain-TCP listener now (loopback/dev + tests), TLS listener via cert/key files at deploy

## does not own
- session content semantics — inner lines are opaque strings; the relay never parses `ClientMessage`/`HostMessage`/records
- the host-authoritative core (`workers/session-host/`) and the client core (`workers/session-client/`) — reused unchanged behind the lane
- session state, saves, persistence policy — the host app owns everything; the relay owns nothing
- `SessionNet` composition (`workers/session-net/` — the relay lane becomes a session shape there in phase-3)
- TLS certificate acquisition (certbot outside the binary; the binary only consumes cert/key paths)

## children-encapsulations
- none

## contents
- `relay_codes.rs` — invite code minting/parsing (room id + room token)
- `relay_protocol.rs` — frame types + length-prefixed codec with cap enforcement
- `relay_server.rs` — `RelayCaps`, room registry, `serve_relay` listener loop, per-connection handling, socket tests
- `bin.rs` — the `thaum-session-relay` binary entrypoint

## dependencies
- `serde` / `serde_json` (frames only; session lines stay opaque strings)
- `workers/session-host/` + `workers/session-client/` (reused unchanged behind the lane, phase-3 wiring)

## exposed interfaces
- `generate_code` / `parse_code` (invite codes)
- `RelayCaps` (frame/member/room/rate limits), `serve_relay`, `RelayServer::shutdown`
- relay frame types (`RelayFrame`) + codec (`read_frame` / `write_frame`)

## interface consumers
- `workers/session-net/` (relay session shape, phase-3 of the relay-transport project plan)
- `orchestration/build-commands/` (boot + panel wiring, phase-4)
- relay operators (the `thaum-session-relay` binary)

## artifacts
- none (zero disk state by design)

## tests
- `relay_codes.rs` inline tests: round-trip, alphabet, wrong-shape rejection
- `relay_server.rs` inline socket tests: host claim + duplicate-host denial, join denials (bad token / no room / user collision / room full), joiner→host and host→all/one routing, frame-cap rejection, join-rate rejection, host disconnect closes the room, member disconnect cleanup

## data
- none (zero disk state is a settled design truth; `data/relay-shape-notes.md` allowed later for non-operational protocol analysis)
