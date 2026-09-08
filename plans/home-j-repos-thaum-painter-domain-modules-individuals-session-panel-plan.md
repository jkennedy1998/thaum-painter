# encapsulation plan — domain/modules/individuals/session-panel (code-shaped join + relay invite display)

linked project plan: `plans/project-thaum-painter-relay-transport-plan.md` (phase-5)

## pre-implementation-note
Settled truth: the session panel renders invite truth verbatim from `SessionNet` — codes when relayed, `ip:port` when LAN. No derivation in UI.

## intent
Make the panel relay-ready without growing UI logic: the join field accepts either shape, and relay invites display + copy exactly like LAN invites already do.

## core design
1. Join field accepts both shapes: one field, hint text names both (`ip:port or code + ENTER`). No lane picker — the shape routes in orchestration.
2. Relay invite display: the in-session invite row renders `invite_addresses()` verbatim; a relay session shows the bare `<room6>-<token10>` code. Already generic — verified, not rebuilt.
3. Copy: `CopyInvite` (buttons row + invite-row click) copies the code; the one-long-lived-clipboard-handle seam from the LAN invite work carries over unchanged.

## interface delta
- none new — `JoinRequested{address}` already carries either shape; the hint text and doc comments now name both shapes

## dependency notes
- `workers/session-net/` invite truth only (`invite_addresses()`)

## consumer notes
- orchestration applies the same actions as today; routing by shape lives there

## test notes
- existing panel tests cover join-field commit, copy routing, and invite rendering; the shape-passthrough behavior is asserted by the join-field test (the field never interprets content)

## data notes
- none
