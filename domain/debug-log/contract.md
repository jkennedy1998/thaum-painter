# domain/debug-log

## purpose
one categorized, level-gated debug logging seam for the whole painter program, shaped after the old system's `src/action_system/debug_logger.ts` (error/warn/info/debug/trace levels, console/file sinks, in-memory buffer) so future debug output lands in one greppable place instead of raw `eprintln!` sprawl.

## owns
- the `DebugLogLevel` ordering (Error < Warn < Info < Debug < Trace)
- the process-global `DebugLogConfig` (enabled flag, minimum level, console sink, optional file sink)
- the `THAUM_PAINTER_DEBUG` env-var toggle (value = minimum level name; unset or `off` = disabled)
- the timestamped line format and the capped in-memory recent-lines buffer
- `debug_log.rs` inline tests

## does not own
- what gets logged or at which level any call site chooses (call sites own that)
- session-state persistence or file-save semantics (`domain/file/storage/`)
- TAI scripts (`domain/tai/`)

## children-encapsulations
- none

## contents
- `debug_log.rs` — levels, config, global seam, line formatting, buffer, tests

## exposed interfaces
### debug_log — process-global leveled logging
send: level + system tag + message (via `log` or the `error/warn/info/debug/trace` helpers)
returns: none (side effects only)
effects: stderr write, optional file append, in-memory buffer push
- `log(level, system, message)` and per-level helpers — the only write seam
- `configure_from_env()` — reads `THAUM_PAINTER_DEBUG`; call once at boot (idempotent)
- `set_config(config)` — programmatic override for tests/UI toggles
- `recent_lines()` — last N buffered lines (cap 500), oldest first
- Warn/Error always reach stderr so operator-visible failures never depend on the toggle; Info/Debug/Trace appear only when enabled and at or below the configured level.

## interface consumers
- `orchestration/entrypoint/` (snapshot conflict recovery, boot env configure)
- future call sites: any domain seam that used to `eprintln!`

## tests
- `debug_log.rs` inline `#[cfg(test)]` module
  - level filtering (below-minimum suppressed, at/above emitted)
  - disabled config swallows Info/Debug/Trace but not Warn/Error console lines
  - env-var parsing (`off`, unknown values, level names)
  - buffer capping and `recent_lines` ordering

## notes
- Deliberately NOT a logging-framework dependency (no `tracing`/`log` crates): one file, zero deps, same shape as the old system's logger.
- The global is a `RwLock<DebugLogConfig>` behind `OnceLock` — no logger is threaded through signatures; call sites are one-line.
- File sink appends per line and never truncates; the operator owns the file path.
