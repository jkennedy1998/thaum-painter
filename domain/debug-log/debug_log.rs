//! Leveled, categorized debug logging for the whole painter program, shaped
//! after the old system's `action_system/debug_logger.ts`: five levels, a
//! console sink, an optional file sink, and a capped in-memory buffer. One
//! process-global config behind a `RwLock` — call sites are one line, nothing
//! threads a logger through signatures.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Log severity ordering: Error is most severe, Trace is noisiest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DebugLogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl DebugLogLevel {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

/// Sink configuration. Warn/Error always reach the console so operator-visible
/// failures never depend on the toggle; everything else is level-gated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugLogConfig {
    pub enabled: bool,
    pub minimum_level: DebugLogLevel,
    pub log_to_console: bool,
    pub file_path: Option<PathBuf>,
}

impl Default for DebugLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            minimum_level: DebugLogLevel::Debug,
            log_to_console: true,
            file_path: None,
        }
    }
}

impl DebugLogConfig {
    /// Builds a config from a `THAUM_PAINTER_DEBUG` value: the level name enables
    /// logging at that minimum, `off` (or nothing) disables.
    pub fn from_env_value(value: Option<&str>) -> Self {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => match DebugLogLevel::parse(value) {
                Some(minimum_level) => Self {
                    enabled: true,
                    minimum_level,
                    ..Self::default()
                },
                // Unknown values (including explicit "off") disable.
                None => Self::default(),
            },
            None => Self::default(),
        }
    }
}

const BUFFER_CAPACITY: usize = 500;

fn config_slot() -> &'static RwLock<DebugLogConfig> {
    static CONFIG: OnceLock<RwLock<DebugLogConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| RwLock::new(DebugLogConfig::default()))
}

fn buffer_slot() -> &'static RwLock<Vec<String>> {
    static BUFFER: OnceLock<RwLock<Vec<String>>> = OnceLock::new();
    BUFFER.get_or_init(|| RwLock::new(Vec::new()))
}

/// Reads `THAUM_PAINTER_DEBUG` into the global config. Idempotent: safe to call
/// once at boot; later `set_config` calls win.
pub fn configure_from_env() {
    let value = std::env::var("THAUM_PAINTER_DEBUG").ok();
    set_config(DebugLogConfig::from_env_value(value.as_deref()));
}

/// Replaces the global config (programmatic override for tests and future UI toggles).
pub fn set_config(config: DebugLogConfig) {
    *config_slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
}

/// Snapshot of the current global config.
pub fn config() -> DebugLogConfig {
    config_slot()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The most recent buffered lines, oldest first (cap `BUFFER_CAPACITY`).
pub fn recent_lines() -> Vec<String> {
    buffer_slot()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Logs one categorized message. `system` is a short tag (`"storage"`,
/// `"stroke"`, `"camera"`) so a session's output can be filtered by subsystem.
pub fn log(level: DebugLogLevel, system: &str, message: &str) {
    let config = config();
    if !config.enabled && level > DebugLogLevel::Warn {
        return;
    }
    if config.enabled && level > config.minimum_level {
        return;
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let line = format!("[{timestamp}] [{}] [{system}] {message}", level.label());

    {
        let mut buffer = buffer_slot()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        buffer.push(line.clone());
        if buffer.len() > BUFFER_CAPACITY {
            buffer.remove(0);
        }
    }

    if config.log_to_console {
        eprintln!("{line}");
    }

    if let Some(file_path) = &config.file_path {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(file_path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

pub fn error(system: &str, message: &str) {
    log(DebugLogLevel::Error, system, message);
}

pub fn warn(system: &str, message: &str) {
    log(DebugLogLevel::Warn, system, message);
}

pub fn info(system: &str, message: &str) {
    log(DebugLogLevel::Info, system, message);
}

pub fn debug(system: &str, message: &str) {
    log(DebugLogLevel::Debug, system, message);
}

pub fn trace(system: &str, message: &str) {
    log(DebugLogLevel::Trace, system, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Mutex, OnceLock};

    /// The seam under test is one process-global config + buffer, and cargo runs
    /// tests in parallel threads — serialize the tests so they don't stomp each
    /// other's config.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct ConfigGuard {
        previous: DebugLogConfig,
    }

    impl ConfigGuard {
        fn with(new_config: DebugLogConfig) -> Self {
            let previous = config();
            set_config(new_config);
            Self { previous }
        }
    }

    impl Drop for ConfigGuard {
        fn drop(&mut self) {
            set_config(self.previous.clone());
        }
    }

    #[test]
    fn level_parsing_accepts_all_names_and_rejects_junk() {
        let _lock = test_lock();
        assert_eq!(DebugLogLevel::parse("trace"), Some(DebugLogLevel::Trace));
        assert_eq!(DebugLogLevel::parse("WARN"), Some(DebugLogLevel::Warn));
        assert_eq!(DebugLogLevel::parse("warning"), Some(DebugLogLevel::Warn));
        assert_eq!(DebugLogLevel::parse("nope"), None);
    }

    #[test]
    fn env_value_enables_at_the_named_level_and_disables_otherwise() {
        let _lock = test_lock();
        let on = DebugLogConfig::from_env_value(Some("info"));
        assert!(on.enabled);
        assert_eq!(on.minimum_level, DebugLogLevel::Info);

        for off in [None, Some(""), Some("off"), Some("gibberish")] {
            let config = DebugLogConfig::from_env_value(off);
            assert!(!config.enabled, "value {off:?} should disable logging");
        }
    }

    fn lines_contain(message: &str) -> bool {
        recent_lines()
            .iter()
            .any(|line| line.ends_with(message))
    }

    #[test]
    fn level_filtering_suppresses_below_minimum() {
        let _lock = test_lock();
        let guard = ConfigGuard::with(DebugLogConfig {
            enabled: true,
            minimum_level: DebugLogLevel::Warn,
            log_to_console: false,
            file_path: None,
        });
        info("test", "should be suppressed");
        assert!(!lines_contain("[test] should be suppressed"));

        warn("test", "should be captured");
        assert!(lines_contain("[test] should be captured"));
        drop(guard);
    }

    #[test]
    fn disabled_config_still_emits_warn_and_error_but_not_lower_levels() {
        let _lock = test_lock();
        let guard = ConfigGuard::with(DebugLogConfig::default());
        debug("test", "suppressed while disabled");
        assert!(!lines_contain("[test] suppressed while disabled"));
        warn("test", "always visible");
        assert!(lines_contain("[test] always visible"));
        drop(guard);
    }

    #[test]
    fn buffer_caps_at_capacity_keeping_the_newest_lines() {
        let _lock = test_lock();
        let guard = ConfigGuard::with(DebugLogConfig {
            enabled: true,
            minimum_level: DebugLogLevel::Trace,
            log_to_console: false,
            file_path: None,
        });
        for _ in 0..(BUFFER_CAPACITY + 25) {
            trace("flood", "line");
        }
        let lines = recent_lines();
        // The cap is respected and the newest line survived.
        assert!(lines.len() <= BUFFER_CAPACITY);
        assert!(lines.last().unwrap().contains("[flood]"));
        drop(guard);
    }
}
