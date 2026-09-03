# /home/j/Repos/thaum-painter/tools/file-dialogs

## purpose
Own the low-level native file-dialog and terminal-prompt plumbing for picking document paths, with no painter-domain truth of its own.

## owns
- native open/save dialog invocation over `rfd`
- terminal fallback prompting when no native dialog is available
- dialog-path normalization rules (document.json parent, slugged file stem)
- backend availability diagnostics for missing dialog selections

## does not own
- painter document locations or file roots (owned by `domain/file/`)
- command-bar session actions (owned by `domain/painter-session/commands/`)
- document load/save semantics (owned by `domain/file/storage/`)

## children-encapsulations
- none

## contents
- `src/lib.rs`
  - dialog helpers

## dependencies
- `rfd`

## exposed interfaces
- `prompt_open_document_root` — native dialog (with terminal fallback) resolving to a document root folder
- `prompt_save_document_root` — native save dialog (with terminal fallback) resolving to a document root folder
- `normalize_open_document_root` — map a picked document.json to its parent folder
- `save_as_root_from_dialog_path` — map a picked save path to a document root folder
- `slugify_file_stem` — lowercase dashed slug from arbitrary text

## interface consumers
- `domain/painter-session/commands/` (command-bar file actions)

## tests
- `file-dialogs`
  - light
  - validates document-root normalization and slugged save-path mapping.

## data
- none

## notes
- kept out of `domain/` because native dialog backends are platform intake, not painter semantics; `domain/` consumes the resolved paths only.
