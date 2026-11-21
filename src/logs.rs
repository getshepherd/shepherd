//! Shepherd's log: a file that outlives the run, for what an app opened from Finder or the Dock would print to a
//! stderr nobody reads. `log`'s records go there, Shepherd's own and gpui's, and a panic with its backtrace; a
//! panic also leaves a note that the next launch shows (`take_crash`).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// A log grown past this is moved aside to `shepherd.old.log` at launch, the one before it dropped.
const ROTATE_AT: u64 = 5 << 20;

/// macOS's ~/Library/Logs, where Console finds it; elsewhere next to the caches.
pub fn dir() -> PathBuf {
    if cfg!(target_os = "macos") { crate::settings::home().join("Library/Logs/Shepherd") } else { crate::settings::cache_dir().join("logs") }
}

pub fn path() -> PathBuf {
    dir().join("shepherd.log")
}

fn crash_note() -> PathBuf {
    dir().join("crashed")
}

struct Logger {
    file: Mutex<File>,
    /// Run from a terminal (`cargo run`): the lines go to it too.
    echo: bool,
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        // gpui and the rest say much at info; theirs are kept from warn up
        m.level() <= if m.target().starts_with("shepherd") { log::Level::Info } else { log::Level::Warn }
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:5} {}: {}\n", now(), r.level(), r.target(), r.args());
        self.write(&line);
    }

    fn flush(&self) {
        let _ = self.file.lock().map(|mut f| f.flush());
    }
}

impl Logger {
    fn write(&self, line: &str) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
        }
        if self.echo {
            eprint!("{line}");
        }
    }
}

static LOGGER: std::sync::OnceLock<Logger> = std::sync::OnceLock::new();

/// Opens the log (rotating it) and installs the logger and the panic hook. Once, at the app's launch; the CLI
/// commands (`shepherd hooks`, `browser`, …) keep printing to their caller's stderr.
pub fn init() {
    let path = path();
    let _ = std::fs::create_dir_all(dir());
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_AT) {
        let _ = std::fs::rename(&path, dir().join("shepherd.old.log"));
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else { return };
    use std::io::IsTerminal;
    let logger = LOGGER.get_or_init(|| Logger { file: Mutex::new(file), echo: std::io::stderr().is_terminal() });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    logger.write(&format!("{} ----- Shepherd {} on {} {}\n", now(), env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let report = format!("{info}\nthread: {thread}\n{}", std::backtrace::Backtrace::force_capture());
        if let Some(logger) = LOGGER.get() {
            logger.write(&format!("{} PANIC {report}\n", now()));
            log::logger().flush();
        }
        // the message, without where: the toast has room for a line
        let message = info.to_string().lines().skip(1).collect::<Vec<_>>().join(" ");
        let _ = std::fs::write(crash_note(), message.chars().take(160).collect::<String>());
        default(info);
    }));
}

/// What the last run panicked with, once: the note goes as it is read.
pub fn take_crash() -> Option<String> {
    let note = std::fs::read_to_string(crash_note()).ok()?;
    let _ = std::fs::remove_file(crash_note());
    Some(note)
}

/// Now in UTC, `2026-11-21 23:40:05Z`. ponytail: UTC, no time zone crate for local time; Console shows both.
fn now() -> String {
    utc(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64)
}

fn utc(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_utc() {
        assert_eq!(super::utc(0), "1970-01-01 00:00:00Z");
        assert_eq!(super::utc(951_782_400 + 3_723), "2000-02-29 01:02:03Z");
    }
}
