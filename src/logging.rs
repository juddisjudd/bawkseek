use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Writes log lines to `LOG_FILE`, or to stderr, at the level named by `LOG_LEVEL`; errors only when unset.
struct Logger {
    level: LevelFilter,
    file: Option<Mutex<File>>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}: {}\n",
            chrono::Local::now().format("%H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        );
        match &self.file {
            Some(file) => {
                if let Ok(mut file) = file.lock() {
                    let _ = file.write_all(line.as_bytes());
                }
            }
            None => eprint!("{line}"),
        }
    }

    fn flush(&self) {}
}

pub fn init() {
    let level = std::env::var("LOG_LEVEL")
        .ok()
        .and_then(|name| name.parse::<Level>().ok())
        .map_or(LevelFilter::Error, |level| level.to_level_filter());
    let file = std::env::var_os("LOG_FILE").and_then(|path| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    });
    if log::set_boxed_logger(Box::new(Logger { level, file })).is_ok() {
        log::set_max_level(level);
    }
}
