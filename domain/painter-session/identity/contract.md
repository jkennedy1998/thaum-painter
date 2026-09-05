# /home/j/Repos/thaum-painter/domain/painter-session/identity

## purpose
Own the painter's stable per-install session identity: the globally-unique `user_id` that keys record ownership, plus the cosmetic display name and presence color.

## owns
- `SessionIdentity` — `user_id`, `display_name`, `presence_color`
- identity generation (random 128-bit `u-{hex}` id from /dev/urandom, best-effort time+pid fallback) and one-time persistence (`load_or_create` never regenerates an existing identity)
- `PRESENCE_CANDIDATE_COLORS` — the hand-picked presence candidates (primaries/secondaries from the indexed palette)
- action-id uniqueness at the minting seam (with `session_document.rs`): minted ids include the `user_id`, so two sessions can mint offline forever without collision (Figma's client-ID-in-object-ID rule)

## does not own
- accounts, auth, or network identity (self-asserted, like Minecraft LAN names)
- the identity file's location (entrypoint-owned path policy) or env-var test overrides
- record ownership logic itself (`sync/` skips foreign records by comparing `user_id`)
- the indexed palette data (`modules/shared/legacy_indexed_palette.rs`)
- presence rendering (future)

## children-encapsulations
- none

## contents
- `identity.rs`
  - `SessionIdentity`, generation, persistence, presence candidates, tests

## dependencies
- `/home/j/Repos/thaum-painter/domain/modules/shared/` (palette lockstep test)

## exposed interfaces
- `SessionIdentity::generate(display_name)` — fresh identity, random id + presence color
- `SessionIdentity::load_or_create(path, display_name)` — load persisted identity or create it exactly once
- `PRESENCE_CANDIDATE_COLORS` — the presence color pick list

## interface consumers
- `orchestration/entrypoint/` (loads identity at boot; `THAUM_SESSION_USER_ID` env stays as the test escape hatch for the user_id)
- `painter-session/session_document.rs` `next_action_id` (bakes user_id into minted action ids)

## artifacts
- none (the identity file itself lives at the entrypoint-chosen runtime path; it is runtime state, not a contract artifact)

## tests
- `identity` inline `#[cfg(test)]` module (light)
  - generated user ids are unique and well-formed
  - persisted identity round-trips and never regenerates
  - presence candidates all come from the indexed palette; assigned color comes from the candidates
  - display name is cosmetic and defaults sanely

## data
- none

## notes
- Source-of-truth from J (2026-09-05): an identity is permanent once created — rotating a user_id would make your own old records foreign in live sync.
- Source-of-truth from J: `display_name` and presence color are cosmetic, editable any time, never used for ownership. Two users with the same display name are fine; the user_id disambiguates.
- Source-of-truth from J: presence colors come from a small manually-picked subset of the indexed palette (primaries and secondaries that read well on both document backgrounds), stored as an RGB triple — not a palette index and not a hash — so the indexed color system can change without breaking existing identities. Expect this list to be revised when the palette system changes.
- Figma reference (figma.com/blog/how-figmas-multiplayer-technology-works): clients mint ids offline by including a unique client id in generated ids; the server orders, timestamps don't arbitrate; identity survives reconnects while connections are disposable. We follow the same shape at identity/action-id level.
- Migration: identities introduced after the first `$USER`-keyed records existed; old records keep their `user_id` strings and replay unchanged. The first identity creation orphans any old `{USER}.json` per-user session state file (one-time persisted-UI reset, accepted by J).
