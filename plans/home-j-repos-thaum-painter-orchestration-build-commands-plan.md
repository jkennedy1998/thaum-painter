# encapsulation plan — orchestration/build-commands (relay boot + shape routing)

linked project plan: `plans/project-thaum-painter-relay-transport-plan.md` (phase-4)

## pre-implementation-note
Settled truth: the panel stays a dumb view; routing by input shape happens here. `THAUM_SESSION_RELAY` explicitly set opts hosting into the relay lane; LAN stays the default free lane.

## intent
Boot and wire the relay lane: env boot variants, panel-action routing (code vs direct address), and relay invite display through the existing session panel.

## core design
1. Env boot: `THAUM_SESSION_RELAY=host:port` → hosting dials the relay (invite code minted at boot); `THAUM_SESSION_RELAY` + `THAUM_SESSION_CODE=<code>` → joins by code. Relay envs win over LAN envs (one process, one lane).
2. Panel-action routing (`apply_session_panel_action`):
   - `HostRequested`: `THAUM_SESSION_RELAY` set → `SessionNet::host_relay`; unset → LAN `host()` (unchanged).
   - `JoinRequested{address}`: contains `:` → LAN join at `ip:port`; otherwise → relay code join. A code join with no relay env dials `session_relay::DEFAULT_RELAY_ADDRESS` (the consumer path — no env juggling for friends).
3. Invite copy: the panel's existing `CopyInvite` copies `invite_addresses()` verbatim — for relay sessions that is the bare code, the whole invite.

## interface delta
- none new: env boot variants + action application only, per the project plan

## dependency notes
- `workers/session-net/` — relay lane + boot parse; consumed, never edited here

## consumer notes
- the entrypoint frame loop: no changes to publish/sync/persist gates

## test notes
- boot parsing and the default-relay fallback live in `session-net`'s boot-parse tests (pure, env-independent)
- panel-action routing is exercised by the live app (thin match over settled shapes); the full end-to-end matrix is phase-7 verification

## data notes
- none
