//! Persistent logs and crash reports, so failures in builds without a console leave a record.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

/// The active log is rotated once at startup above this size, and stops growing past it.
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CRASH_BYTES: u64 = 1024 * 1024;

/// Log to stderr and, when the per-user log directory is writable, to `pecan.log` there.
///
/// `RUST_LOG` keeps overriding the filter. Panics append a report with a backtrace to
/// `crash.log`, because release builds abort and Windows builds have no console.
pub(crate) fn install() {
    // The binary crate is named `Pecan`, so its events use that target.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,Pecan=info"));
    let directory = pecan_config::log_directory();
    let file = directory.as_deref().and_then(|directory| {
        open_log(directory)
            .map_err(|error| {
                eprintln!(
                    "Pecan cannot open its log in {}: {error}",
                    directory.display()
                )
            })
            .ok()
    });
    let file_layer = file.map(|file| {
        fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(Bounded {
                written: file.metadata().map_or(0, |metadata| metadata.len()),
                file,
            }))
    });
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(io::stderr))
        .with(file_layer)
        .try_init();
    install_panic_hook(directory);
}

fn open_log(directory: &Path) -> io::Result<File> {
    fs::create_dir_all(directory)?;
    let path = directory.join("pecan.log");
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES / 2) {
        let _ = fs::rename(&path, directory.join("pecan.log.1"));
    }
    append(&path)
}

fn append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Paths of opened documents are logged; keep them private to the user.
        options.mode(0o600);
    }
    options.open(path)
}

/// Drop log lines beyond the size budget instead of filling the disk.
struct Bounded {
    file: File,
    written: u64,
}
impl Write for Bounded {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.written >= MAX_LOG_BYTES {
            return Ok(buffer.len());
        }
        let count = self.file.write(buffer)?;
        self.written += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn install_panic_hook(directory: Option<PathBuf>) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Write directly: a panic inside the log writer would deadlock on its mutex.
        if let Some(directory) = &directory {
            let _ = write_crash(directory, &info.to_string());
        }
        default(info);
    }));
}

fn write_crash(directory: &Path, message: &str) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let path = directory.join("crash.log");
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_CRASH_BYTES) {
        let _ = fs::remove_file(&path);
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let thread = std::thread::current();
    let report = format!(
        "=== Pecan {} crashed at unix time {seconds} on {} ({}) ===\nthread: {}\n{message}\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        thread.name().unwrap_or("<unnamed>"),
        std::backtrace::Backtrace::force_capture(),
    );
    let mut file = append(&path)?;
    file.write_all(report.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_reports_append_and_logs_rotate_within_their_budgets() {
        let directory = tempfile::tempdir().unwrap();
        write_crash(directory.path(), "first").unwrap();
        write_crash(directory.path(), "second").unwrap();
        let report = fs::read_to_string(directory.path().join("crash.log")).unwrap();
        assert!(report.contains("first") && report.contains("second"));
        assert!(report.contains(env!("CARGO_PKG_VERSION")));

        let log = directory.path().join("pecan.log");
        fs::write(&log, vec![b'x'; (MAX_LOG_BYTES / 2 + 1) as usize]).unwrap();
        let file = open_log(directory.path()).unwrap();
        assert_eq!(
            file.metadata().unwrap().len(),
            0,
            "an oversized log starts over"
        );
        assert!(directory.path().join("pecan.log.1").exists());

        let mut bounded = Bounded {
            file,
            written: MAX_LOG_BYTES,
        };
        assert_eq!(bounded.write(b"dropped").unwrap(), 7);
        assert_eq!(fs::metadata(&log).unwrap().len(), 0);
    }
}
