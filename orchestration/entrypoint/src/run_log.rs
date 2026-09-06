//! Host-owned run-log boot wiring for the painter.
//!
//! The renderer's `debug_log` seam owns sinks; this module owns where they
//! point and what wraps the process. One folder per boot under
//! `context/debug-logging/<UTC-stamp>[-NN]/` holds the always-on `run.log`
//! (renderer + painter lines in one stream) and the per-run `perf.jsonl`.
//! Boot culls the folder set to the newest `KEEP_RUN_LOG_DIRS` so the last
//! few runs are always available for post-crash support without growing
//! forever. A panic hook routes panic messages + backtraces into `run.log`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use thaum_renderer_domain::debug_log;

/// How many boot-stamped run-log folders survive culling.
const KEEP_RUN_LOG_DIRS: usize = 5;

/// Prepares the per-run log folder, configures the debug-log sink into it
/// (`THAUM_DEBUG` overrides the default Info floor), installs the panic hook,
/// and returns the folder path for run-scoped artifacts (perf log).
///
/// Best-effort: if the folder cannot be created, logging falls back to the
/// env config alone and `None` is returned.
pub fn boot(painter_root: &Path) -> Option<PathBuf> {
    let run_dir = prepare_run_log_dir(painter_root).ok();

    let mut config = debug_log::DebugLogConfig::from_env_value(
        std::env::var("THAUM_DEBUG").ok().as_deref(),
    );
    if !config.enabled {
        // Always-on run log: Info floor to the file when no env override asks
        // for something else. Warn/Error still reach the console either way.
        config = debug_log::DebugLogConfig {
            enabled: true,
            minimum_level: debug_log::DebugLogLevel::Info,
            ..debug_log::DebugLogConfig::default()
        };
    }
    config.file_path = run_dir.as_ref().map(|dir| dir.join("run.log"));
    debug_log::set_config(config);

    install_panic_hook();

    // Boot header: guarantees run.log exists with identity info even when a
    // run is otherwise clean, so "send me the newest log folder" always has
    // something to grab and the log self-identifies (version/os/arch).
    debug_log::info(
        "boot",
        &format!(
            "thaum-painter {} run started; os={} arch={}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    run_dir
}

/// Creates `context/debug-logging/<stamp>[-NN]` for this boot and culls older
/// folders down to `KEEP_RUN_LOG_DIRS`. The `-NN` suffix dedupes same-second
/// relaunches (test-restart loops) without clobbering.
fn prepare_run_log_dir(painter_root: &Path) -> io::Result<PathBuf> {
    let root = painter_root.join("context/debug-logging");
    fs::create_dir_all(&root)?;

    let base = utc_boot_stamp(SystemTime::now());
    let dir = if !root.join(&base).exists() {
        root.join(base)
    } else {
        (2..=99)
            .map(|n| root.join(format!("{base}-{n:02}")))
            .find(|candidate| !candidate.exists())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "no free run-log folder slot this second",
                )
            })?
    };
    fs::create_dir_all(&dir)?;

    cull_old_run_log_dirs(&root);
    Ok(dir)
}

/// Keeps the newest `KEEP_RUN_LOG_DIRS` boot-stamped folders (names sort
/// chronologically) and removes older ones. Unrecognized entries are left
/// alone — culling never touches something it did not create.
fn cull_old_run_log_dirs(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut stamps: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| is_run_log_stamp(name))
        .collect();
    stamps.sort();
    while stamps.len() > KEEP_RUN_LOG_DIRS {
        let oldest = stamps.remove(0);
        let _ = fs::remove_dir_all(root.join(oldest));
    }
}

/// Recognizes `YYYY-MM-DD-HH-MM-SS` and its `-NN` suffixed forms.
fn is_run_log_stamp(name: &str) -> bool {
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() != 6 && parts.len() != 7 {
        return false;
    }
    let fields: Option<Vec<u32>> = parts[..6].iter().map(|field| field.parse().ok()).collect();
    let Some(fields) = fields else {
        return false;
    };
    let [year, month, day, hour, minute, second] = fields.as_slice() else {
        return false;
    };
    let (year, month, day, hour, minute, second) =
        (*year, *month, *day, *hour, *minute, *second);
    let base_valid = year >= 1000
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 60;
    if parts.len() == 7 {
        return base_valid && parts[6].parse::<u32>().map(|n| n <= 99).unwrap_or(false);
    }
    base_valid
}

/// `YYYY-MM-DD-HH-MM-SS` in UTC from a system time (no chrono dependency).
fn utc_boot_stamp(time: SystemTime) -> String {
    let millis = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let seconds = millis / 1000;
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}-{:02}-{:02}-{:02}",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60
    )
}

/// Days-since-epoch to (year, month, day) — Howard Hinnant's civil-from-days.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let mp = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Routes panics into the shared log seam: message, location, and a forced
/// backtrace land in `run.log` (and the console) before the process dies.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let payload = if let Some(text) = info.payload().downcast_ref::<&str>() {
            (*text).to_string()
        } else if let Some(text) = info.payload().downcast_ref::<String>() {
            text.clone()
        } else {
            "unknown panic payload".to_string()
        };
        let backtrace = std::backtrace::Backtrace::force_capture();
        debug_log::error(
            "panic",
            &format!("{payload} at {location}\n{backtrace}"),
        );
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_is_utc_yyyy_mm_dd_hh_mm_ss() {
        // 2026-09-07 14:03:11 UTC
        let time = UNIX_EPOCH + std::time::Duration::from_millis(1_788_789_791_000);
        assert_eq!(utc_boot_stamp(time), "2026-09-07-14-03-11");
        assert_eq!(utc_boot_stamp(UNIX_EPOCH), "1970-01-01-00-00-00");
    }

    #[test]
    fn stamp_recognizer_accepts_stamp_forms_and_rejects_other_names() {
        assert!(is_run_log_stamp("2026-09-07-14-03-11"));
        assert!(is_run_log_stamp("2026-09-07-14-03-11-07"));
        assert!(!is_run_log_stamp("not-a-stamp"));
        assert!(!is_run_log_stamp("9999-99-99-99-99-99"));
        assert!(!is_run_log_stamp("2026-09-07-14-03-11-extra-more"));
    }

    #[test]
    fn prepare_creates_deduped_folder_and_culls_to_the_cap() {
        let root = std::env::temp_dir().join(format!(
            "painter-run-log-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let painter_root = root.join("painter");
        fs::create_dir_all(&painter_root).unwrap();

        // Pre-seed KEEP_RUN_LOG_DIRS + 1 older folders so this boot's folder
        // pushes the oldest one out.
        let log_root = painter_root.join("context/debug-logging");
        for (index, stamp) in [
            "2026-09-01-00-00-00",
            "2026-09-02-00-00-00",
            "2026-09-03-00-00-00",
            "2026-09-04-00-00-00",
            "2026-09-05-00-00-00",
            "2026-09-06-00-00-00",
        ]
        .into_iter()
        .enumerate()
        {
            fs::create_dir_all(log_root.join(format!("{stamp}-{index:02}"))).unwrap();
        }

        let dir = prepare_run_log_dir(&painter_root).unwrap();
        assert!(dir.join("run.log").parent().unwrap().is_dir());

        let mut survivors: Vec<String> = fs::read_dir(&log_root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        survivors.sort();
        assert_eq!(survivors.len(), KEEP_RUN_LOG_DIRS);
        assert!(
            !survivors.iter().any(|name| name.starts_with("2026-09-01")),
            "oldest folder should be culled: {survivors:?}"
        );
        assert!(
            survivors
                .last()
                .unwrap()
                .starts_with(&utc_boot_stamp(SystemTime::now())),
            "newest folder is this boot's: {survivors:?}"
        );

        // Same-second relaunch dedupes rather than reusing the folder.
        let second = prepare_run_log_dir(&painter_root).unwrap();
        assert_ne!(dir, second);
        assert!(dir.exists(), "first boot folder survives the second boot");

        let _ = fs::remove_dir_all(&root);
    }
}
