# thaum-painter/domain/painter-tools

## purpose
Own painter tool registration as one encapsulation: a registry of tool descriptors plus one folder per tool, so adding a tool is one folder plus one registry line and every registration consumer reads the same truth.

## owns
- the `ToolDescriptor` registration shape: id, label, icon, description (toolbox tooltip copy), tool-specific property rows, select action, default hotkey
- the `RegisteredTool` seam each `individuals/` tool folder implements
- the single `ALL_TOOLS` registry list and its lookup helpers
- drift tests asserting registry <-> tool folders <-> `PaintTool` enum <-> tai hotkeys

## does not own
- session behavior of tools (edit/select semantics), still owned by `painter-session/tool-state` match arms until per-tool behavior migration
- pure data transforms, owned by `painter-operations/`
- the properties panel UI, owned by `modules/individuals/hand-settings/`
- the toolbox UI, owned by `modules/individuals/toolbox/`
- binding map storage, owned by `tai/`

## children-encapsulations
- `shared/`
  - default
- `individuals/`
  - default

## contents
- `contract.md`
  - painter-tools contract
- `painter_tools.rs`
  - encapsulation module root
- `registry.rs`
  - `ToolDescriptor`, `RegisteredTool`, `ALL_TOOLS`, lookup helpers, registry tests
- `shared/`
  - cross-tool behavior rules and computations written once

## dependencies
- none (leaf; consumers depend on it, it depends on nothing)

## exposed interfaces
### tool registry
send: an optional tool id
returns: every registered tool descriptor, or the one matching that id
effects: none
via: `all`, `by_id`, `require_by_id`

### tool registration
send: none
returns: one descriptor per tool folder
effects: none
via: each `individuals/<tool>/` type implementing `RegisteredTool`

## interface consumers
- `painter-session/tool-state/` (enum <-> registry bridge, property manifest)
- `user-state/user-session-state/` (persistence names)
- `modules/individuals/hand-settings/` (tool labels)
- `modules/individuals/toolbox/` consumers build the `ToolDef` list from descriptors
- `orchestration/entrypoint/` (toolbox registration, hotkey action mapping)
- `tai/` (hotkey drift tests)

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `registry.rs` and each tool file
  - light
  - validates unique non-empty ids, select-action/hotkey pairing, folder <-> registry parity, and per-tool descriptor truth
- `tai/` drift test
  - light
  - validates every registered tool hotkey matches the declared bindings

## data
- none

## notes
- source-of-truth from J: tools should live as encapsulations like workshops do — shared and individuals folders — so a registry can be derived automatically and contract cascade checks (jobo-style) can run over them.
- source-of-truth from J: logic should be written in one place and reused through standard seams, never rewritten per tool; `shared/` hosts that cross-tool layer (selection behavior rules now, geometry/line/shape computations as tools grow).
- lean + non-migratory: registration truth moved here first; behavior migration happens per tool later, one folder at a time.
- adding a new tool: new folder under `individuals/` (contract.md + `<tool>_tool.rs`) + one `ToolDescriptor` line in `registry.rs` + one `PaintTool` arm in tool-state; property rows, label, icon, persistence name, and hotkey then flow from the descriptor.
- clearing is not a tool registration: any ordinary paint tool clears when its hand has the graphics picker's clear character equipped.
