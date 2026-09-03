# /home/j/Repos/thaum-painter/domain/painter-tools/individuals/text

## purpose
Own the text tool's registration truth as one tool encapsulation.

## owns
- the `TextTool` registration type resolving text's `ToolDescriptor` (id "text", label, icon, no property rows yet, no hotkey yet)
- text's future session behavior home (typing behavior still lives in `painter-session/tool-state/` plus the session's `TextEntryState` bridge)

## does not own
- the registry list
- text stamping/layout semantics, owned by `painter-operations/text/` (pure) and the session text-entry bridge until migration
- text layout option state, owned by `painter-session/tool-state/`

## children-encapsulations
- none

## contents
- `contract.md`
  - text tool contract
- `text_tool.rs`
  - `TextTool` `RegisteredTool` impl + descriptor test

## dependencies
- the registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `text_tool.rs`
  - light
  - validates text's descriptor truth

## data
- none

## notes
- open gap: layout options (spacing/charlead/enterlead/enterspace/space-replace) live default-only in tool-state with no UI; when surfaced, they become text's property rows via the descriptor.
