# encapsulation plan — workers/session-net (relay lane joins the session shapes)

linked project plan: `plans/project-thaum-painter-relay-transport-plan.md` (phase-3)

## pre-implementation-note
Settled truth: a relay session is indistinguishable from a LAN session to the entrypoint — same publish/sync/rejoin/reseed semantics, different dial target. `SessionNet` stays the ONLY session seam the entrypoint touches.

## intent
Add the relay lane as a third session shape beside `host()`/`join()`, wiring the unchanged host/joiner cores through `session-relay`'s client lane.

## core design
1. `SessionNet::host_relay(relay_address, snapshot_source, user, seed_records)` — spawns the relay host bridge (`spawn_relay_host_bridge`), mints the invite code, and drives the same `SessionHost` core the LAN host uses. Dropping the handle drops the session.
2. `SessionNet::join_relay(relay_address, code, user)` — starts a `RelayClientBridge` (loopback listener), then runs the UNCHANGED `SessionClient` connect/rejoin path against `127.0.0.1:<ephemeral>`. Auto-rejoin, keepalive, reseed convergence come from the existing client path.
3. Invite truth: `invite_addresses()` returns exactly one entry for relay sessions — the minted code, rendered verbatim by the panel. LAN sessions keep their interface list.
4. Boot parse: `THAUM_SESSION_RELAY` env wins over LAN envs (one process, one lane); `THAUM_SESSION_CODE` with no relay env dials `session_relay::DEFAULT_RELAY_ADDRESS` (consumer path); LAN shapes unchanged.
5. Rejoin over relay: the bridge re-dials per `SessionClient::connect` — the existing backoff path works unchanged because the loopback address never changes.

## interface delta
- `SessionNet::host_relay(...) -> Result<SessionNet>`; `SessionNet::join_relay(...) -> Result<(SessionNet, SharedDocumentFile)>`
- `SessionNetBoot::RelayHost(String)` / `RelayJoin { relay, code }`
- no change to publish/sync/persist gates or the frame loop

## dependency notes
- `workers/session-relay/` — the client lane + codes; consumed, never edited here
- `workers/session-host/`, `workers/session-client/` — unchanged reuse

## consumer notes
- orchestration `build-commands`: env boot variants + panel-action shape routing
- session panel: renders `invite_addresses()` verbatim (codes or ip:port)

## test notes
- relay-mode convergence (stroke visibility both directions)
- rejoin after relay link drop; reseed rebuilds joiners on the new document
- invite truth returns exactly the code; boot parse covers relay host/join and the default-relay fallback

## data notes
- none
