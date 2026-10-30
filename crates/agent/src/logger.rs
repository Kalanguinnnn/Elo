//! Tiny file logger (rotates at 5 MB). Never logs secrets: the engine only
//! passes redacted messages.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct FileLogger {
    path: PathBuf,
    file: Mutex<Option<std::fs::File>>,
}

impl FileLogger {
    pub fn install(path: PathBuf) {
        let logger = Box::leak(Box::new(FileLogger { path, file: Mutex::new(None) }));
        if log::set_logger(logger).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    }

    fn open(&self) -> Option<std::fs::File> {
        if std::fs::metadata(&self.path).map(|m| m.len() > 5 * 1024 * 1024).unwrap_or(false) {
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
        }
        std::fs::OpenOptions::new().create(true).append(true).open(&self.path).ok()
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let mut g = self.file.lock().unwrap();
        if g.is_none() {
            *g = self.open();
        }
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{} {:5} {}", chrono_like_now(), r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

fn chrono_like_now() -> String {
    continuity_core::util::ts_to_rfc3339(continuity_core::util::now_ms())
}
