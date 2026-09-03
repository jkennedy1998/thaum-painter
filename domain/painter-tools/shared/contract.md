# /home/j/Repos/thaum-painter/domain/painter-tools/shared

## purpose
Own cross-tool behavior rules and computations written once and consumed through a standard seam — so logic is never rewritten per tool.

## owns
- per-tool selection-surface behavior: which selection mode a tool forces when its hand edits the selection surface (`ToolSelectionBehavior` + `selection_behavior`)
- future residents: geometry, line/shape computations, and other repeated cross-tool logic as tools accumulate

## does not own
- the registry list, owned by the parent `painter-tools/registry.rs`
- per-tool behavior that belongs to exactly one tool (those live in that tool's `individuals/` folder as behavior migrates)
- selection mode semantics themselves, owned by `painter-session/selection/` — this seam decides *which* mode applies, it does not define modes

## children-encapsulations
- none

## contents
- `contract.md`
  - shared contract
- `selection_rules.rs`
  - `ToolSelectionBehavior` (UseCurrentMode / ForceSubtract) with a generic `resolve`, plus `selection_behavior(tool_id)` lookup and tests

## dependencies
- the parent registry seam (`domain/painter-tools/`)

## exposed interfaces
### selection behavior
send: a tool's registration id
returns: how that tool behaves when its hand edits the selection surface
effects: none
via: `selection_behavior`, `ToolSelectionBehavior::resolve(current, forced)`

## interface consumers
- `painter-session/tool-state/` (selection application)
- `orchestration/entrypoint/` (selection stroke start for either hand)

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `selection_rules.rs`
  - light
  - validates erase forces subtract, other tools use the current mode, resolve picks the matching arm, and every registered tool resolves a behavior

## data
- none

## notes
- source-of-truth from J: the erase-subtract rule was written three times; shared seams like this one — plus future geometry/line/shape computation seams — should exist so logic is written in one place and reused through standard seams.
- the resolve seam takes the caller's mode type as parameters, keeping painter-tools dependency-free; the rule (which tool forces what) is the single source here.
- when a tool's behavior migrates into its `individuals/` folder, tool-specific rules move there and only genuinely cross-tool logic stays in `shared/`.
