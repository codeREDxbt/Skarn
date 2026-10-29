//! Crash and fatal-error logging for the desktop shell.
//!
//! Panic records go to `<app-data>/logs/selora.log` — `%APPDATA%\selora-ide\`
//! `logs\selora.log` in production, or `$SELORA_SETTINGS_DIR/logs` when that
//! override is set. The file rotates by size (`selora.log`, `selora.log.1`,
//! … up to [`KEEP_LOG_FILES`] files) so a crash loop cannot fill the disk.
//!
//! Every operation here is best effort. The panic hook never unwinds its own
//! frame (`catch_unwind` + a re-entrancy guard), never calls `unwrap`, and
//! ignores I/O errors: a missing or unwritable log directory must not turn one
//! crash into an abort loop.
//!
//! Deliberate limitation: the hook does **not** try to save dirty editor
//! buffers. The workspace, buffer store and hot-exit writer are not unwind-
//! safe, and taking their locks from a panicking thread can deadlock or
//! corrupt state. The record says so explicitly; recovery relies on the
//! normal save path and, once wired, the hot-exit snapshot.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Directory under the app-data root that holds the rotating log files.
pub const LOG_DIR_NAME: &str = "logs";

/// Current log file. Rotated siblings are `selora.log.1`, `selora.log.2`, …
pub const LOG_FILE_NAME: &str = "selora.log";

/// Rotate before an append would push the current file past this size.
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

/// Total files kept by rotation (the current file plus its rotated siblings).
pub const KEEP_LOG_FILES: usize = 5;

/// Guards against a panic raised inside the hook itself. The hook must never
/// recurse into the log writer.
static HOOK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Resolve the log directory without failing: the settings directory when the
/// app-data root is available, else a temp-dir fallback. The caller still has
/// to create it (see [`ensure_log_dir`]); resolution alone touches no disk.
pub fn log_dir() -> PathBuf {
    let base = selora_settings::Settings::settings_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join(selora_settings::CONFIG_DIR));
    base.join(LOG_DIR_NAME)
}

/// Create the log directory and return it. An unwritable location is an
/// ordinary `io::Error` for the "Open Logs" command; the panic hook ignores
/// it.
pub fn ensure_log_dir() -> io::Result<PathBuf> {
    let dir = log_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Install the rotating panic hook. Replaces the previous hook (usually the
/// default stderr printer) and still calls it, so debug builds keep the
/// console output.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A panic inside a panic hook aborts the process. catch_unwind keeps a
        // formatting/I-O slip from doing that; the flag stops re-entry if the
        // process panics while the hook is already running.
        if !HOOK_ACTIVE.swap(true, Ordering::SeqCst) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                record_panic_to(&log_dir(), info);
            }));
            HOOK_ACTIVE.store(false, Ordering::SeqCst);
        }
        previous(info);
    }));
}

/// Record a fatal, non-panicking shutdown (for example the Tauri event loop
/// returning an error). Appends to the same rotating file; never panics.
pub fn record_fatal(message: &str) {
    let record = format!(
        "----- Selora fatal {} -----\nMessage: {message}\n\
         No editor buffers were saved from the fatal handler.\n\
         ----- end fatal -----\n\n",
        format_timestamp(SystemTime::now())
    );
    let _ = append_record(&log_dir(), &record);
}

/// Format one panic record. Pure so the hook's output shape is testable
/// without inducing a real panic.
pub fn format_panic_record(
    message: &str,
    location: Option<&str>,
    thread: &str,
    timestamp: &str,
    backtrace: &str,
) -> String {
    let backtrace = if backtrace.trim().is_empty() {
        "<backtrace unavailable>"
    } else {
        backtrace
    };
    format!(
        "----- Selora panic {timestamp} -----\n\
         Thread: {thread}\n\
         Location: {location}\n\
         Message: {message}\n\
         Backtrace:\n\
         {backtrace}\n\
         Note: unsaved editor buffers were not saved from the panic hook \
         (touching workspace state while unwinding is unsafe).\n\
         ----- end panic -----\n\n",
        location = location.unwrap_or("<unknown>"),
    )
}

/// Append `record` to [`LOG_FILE_NAME`] in `dir`, rotating first when the
/// current file has reached [`MAX_LOG_BYTES`]. Errors are returned for the
/// hook to ignore.
pub fn append_record(dir: &Path, record: &str) -> io::Result<()> {
    append_with_limits(
        dir,
        LOG_FILE_NAME,
        record.as_bytes(),
        MAX_LOG_BYTES,
        KEEP_LOG_FILES,
    )
}

/// Hook body, parameterized by directory and panic metadata for tests.
fn record_panic_to(dir: &Path, info: &std::panic::PanicHookInfo<'_>) {
    let message = panic_message(info);
    let location = info.location().map(|location| {
        format!(
            "{}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        )
    });
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let timestamp = format_timestamp(SystemTime::now());
    // force_capture ignores RUST_BACKTRACE so the log always names a frame.
    let backtrace = format!("{:?}", std::backtrace::Backtrace::force_capture());
    let record = format_panic_record(
        &message,
        location.as_deref(),
        thread_name,
        &timestamp,
        &backtrace,
    );
    let _ = append_record(dir, &record);
}

/// The panic payload as text. Non-string payloads are labeled, never downcast
/// with `unwrap`.
fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Append one record, rotating when the next write would exceed `max_bytes`.
/// `keep` counts the current file, so the siblings are `.1` … `.keep-1`.
fn append_with_limits(
    dir: &Path,
    file_name: &str,
    bytes: &[u8],
    max_bytes: u64,
    keep: usize,
) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let base = dir.join(file_name);
    let current_len = fs::metadata(&base).map(|meta| meta.len()).unwrap_or(0);
    // A single oversized record still gets written; rotation then keeps the
    // next one from appending to it.
    if current_len > 0 && current_len.saturating_add(bytes.len() as u64) > max_bytes {
        rotate(dir, file_name, keep)?;
    }

    let mut file = OpenOptions::new().create(true).append(true).open(&base)?;
    file.write_all(bytes)?;
    if !bytes.ends_with(b"\n") {
        file.write_all(b"\n")?;
    }
    Ok(())
}

/// `selora.log` → `selora.log.1` → … → `selora.log.<keep-1>`, deleting the
/// oldest. A missing source is not an error (the first rotation may find no
/// sibling at a given index).
fn rotate(dir: &Path, file_name: &str, keep: usize) -> io::Result<()> {
    let keep = keep.max(1);
    let base = dir.join(file_name);
    if keep == 1 {
        return match fs::remove_file(&base) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }

    let sibling = |index: usize| dir.join(format!("{file_name}.{index}"));
    remove_if_present(&sibling(keep - 1))?;
    for index in (1..keep - 1).rev() {
        match fs::rename(sibling(index), sibling(index + 1)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    fs::rename(&base, sibling(1))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// ISO-8601 UTC with milliseconds. Hand-rolled to avoid a chrono dependency
/// on the panic path.
pub fn format_timestamp(time: SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(UNIX_EPOCH) else {
        return "unknown-time".to_string();
    };
    let seconds = elapsed.as_secs();
    let millis = elapsed.subsec_millis();
    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Howard Hinnant's `civil_from_days` (days since 1970-01-01).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn crash_panic_record_has_timestamp_thread_location_and_backtrace() {
        let record = format_panic_record(
            "synthetic panic message",
            Some("src/main.rs:42:9"),
            "worker-7",
            "2026-10-29T12:34:56.789Z",
            "   0: selora_desktop::crash_log::tests",
        );

        assert!(record.contains("2026-10-29T12:34:56.789Z"));
        assert!(record.contains("Thread: worker-7"));
        assert!(record.contains("Location: src/main.rs:42:9"));
        assert!(record.contains("Message: synthetic panic message"));
        assert!(record.contains("0: selora_desktop::crash_log::tests"));
        assert!(record.contains("not saved from the panic hook"));
        assert!(record.ends_with("\n\n"));
    }

    #[test]
    fn crash_panic_record_tolerates_a_missing_location_and_backtrace() {
        let record = format_panic_record("boom", None, "<unnamed>", "now", "");

        assert!(record.contains("Location: <unknown>"));
        assert!(record.contains("<backtrace unavailable>"));
        assert!(record.contains("Message: boom"));
    }

    #[test]
    fn crash_rotation_names_files_and_keeps_only_the_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let max_bytes = 150;
        let keep = 3;
        let body = "x".repeat(90);

        for index in 0..6 {
            let record = format!("record-{index}-{body}");
            append_with_limits(dir.path(), "selora.log", record.as_bytes(), max_bytes, keep)
                .expect("append");
        }

        let current = fs::read_to_string(dir.path().join("selora.log")).expect("current log");
        let one = fs::read_to_string(dir.path().join("selora.log.1")).expect("first rotation");
        let two = fs::read_to_string(dir.path().join("selora.log.2")).expect("second rotation");

        assert!(current.contains("record-5-"));
        assert!(one.contains("record-4-"));
        assert!(two.contains("record-3-"));
        assert!(!dir.path().join("selora.log.3").exists());
        // The count is `keep` files total, never more.
        let files = fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .count();
        assert_eq!(files, keep);
    }

    #[test]
    fn crash_rotation_retains_a_single_file_when_keep_is_one() {
        let dir = tempfile::tempdir().expect("tempdir");

        append_with_limits(dir.path(), "selora.log", b"one", 4, 1).expect("first");
        append_with_limits(dir.path(), "selora.log", b"two", 4, 1).expect("second");

        let current = fs::read_to_string(dir.path().join("selora.log")).expect("log");
        assert_eq!(current, "two\n");
        assert!(!dir.path().join("selora.log.1").exists());
    }

    #[test]
    fn crash_unwritable_directory_returns_an_error_without_panicking() {
        let root = tempfile::tempdir().expect("tempdir");
        let blocker = root.path().join("blocked");
        fs::write(&blocker, "not a directory").expect("write blocker");
        let unusable = blocker.join("logs");

        let result = std::panic::catch_unwind(|| {
            append_with_limits(&unusable, "selora.log", b"record", 1024, 2)
        });

        let outcome = result.expect("the log writer must not panic");
        assert!(outcome.is_err());
    }

    #[test]
    fn crash_timestamp_is_iso_utc() {
        assert_eq!(format_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_timestamp(UNIX_EPOCH + Duration::from_millis(1_234)),
            "1970-01-01T00:00:01.234Z"
        );
    }
}
