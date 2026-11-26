//! CSV logging: one file per cook, written with the `csv` crate.

use std::fs::File;
use std::path::PathBuf;

use chrono::{DateTime, Local};
use csv::Writer;

use crate::clock::AppInstant;
use crate::model::celsius_to_fahrenheit;

/// Header row for a cook CSV.
const HEADER: [&str; 12] = [
    "timestamp",
    "elapsed_s",
    "p1_c",
    "p2_c",
    "p3_c",
    "p4_c",
    "p1_f",
    "p2_f",
    "p3_f",
    "p4_f",
    "battery_pct",
    "event",
];

/// Appends readings and events to a per-cook CSV file.
#[derive(Debug)]
pub struct CsvLogger {
    enabled: bool,
    dir: PathBuf,
    interval_s: u32,
    path: Option<PathBuf>,
    writer: Option<Writer<File>>,
    last_write: Option<AppInstant>,
}

impl CsvLogger {
    /// Create a logger; the file is created lazily on the first row.
    #[must_use]
    pub fn new(enabled: bool, dir: PathBuf, interval_s: u32) -> Self {
        Self {
            enabled,
            dir,
            interval_s: interval_s.max(1),
            path: None,
            writer: None,
            last_write: None,
        }
    }

    /// Update settings; changing the directory starts a new file.
    pub fn set_config(&mut self, enabled: bool, dir: PathBuf, interval_s: u32) {
        if dir != self.dir {
            self.writer = None;
            self.path = None;
        }
        self.enabled = enabled;
        self.dir = dir;
        self.interval_s = interval_s.max(1);
    }

    /// Whether logging is on.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The current file name, once a row has been written.
    #[must_use]
    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
    }

    /// Whether a periodic sample is due.
    #[must_use]
    pub fn due(&self, at: AppInstant) -> bool {
        self.enabled
            && self
                .last_write
                .is_none_or(|last| at.since(last) >= f64::from(self.interval_s))
    }

    /// Append a row (used for both samples and events).
    pub fn write_row(
        &mut self,
        at: AppInstant,
        wall: DateTime<Local>,
        temps_c: &[Option<f32>],
        battery: Option<u8>,
        event: &str,
    ) {
        if !self.enabled {
            return;
        }
        if let Err(err) = self.write_inner(at, wall, temps_c, battery, event) {
            tracing::warn!("csv write failed: {err}");
            self.enabled = false;
        }
        self.last_write = Some(at);
    }

    /// Flush buffered rows.
    pub fn flush(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }
    }

    fn write_inner(
        &mut self,
        at: AppInstant,
        wall: DateTime<Local>,
        temps_c: &[Option<f32>],
        battery: Option<u8>,
        event: &str,
    ) -> Result<(), String> {
        self.ensure_file(wall)?;
        let Some(writer) = self.writer.as_mut() else {
            return Ok(());
        };
        let cell = |index: usize, fahrenheit: bool| {
            temps_c
                .get(index)
                .copied()
                .flatten()
                .map_or_else(String::new, |value| {
                    if fahrenheit {
                        format!("{:.1}", celsius_to_fahrenheit(value))
                    } else {
                        format!("{value:.1}")
                    }
                })
        };
        let record = [
            wall.format("%Y-%m-%d %H:%M:%S").to_string(),
            format!("{:.0}", at.secs()),
            cell(0, false),
            cell(1, false),
            cell(2, false),
            cell(3, false),
            cell(0, true),
            cell(1, true),
            cell(2, true),
            cell(3, true),
            battery.map_or_else(String::new, |pct| pct.to_string()),
            event.to_string(),
        ];
        writer
            .write_record(record.iter())
            .map_err(|err| err.to_string())?;
        writer.flush().map_err(|err| err.to_string())
    }

    fn ensure_file(&mut self, wall: DateTime<Local>) -> Result<(), String> {
        if self.writer.is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir).map_err(|err| err.to_string())?;
        let name = format!("cook-{}.csv", wall.format("%Y-%m-%d-%H%M"));
        let path = self.dir.join(name);
        let file = File::options()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|err| err.to_string())?;
        let empty = file.metadata().map_or(true, |meta| meta.len() == 0);
        let mut writer = Writer::from_writer(file);
        if empty {
            writer
                .write_record(HEADER.iter())
                .map_err(|err| err.to_string())?;
        }
        writer.flush().map_err(|err| err.to_string())?;
        self.path = Some(path);
        self.writer = Some(writer);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn writes_header_and_rows() {
        let dir = std::env::temp_dir().join(format!("bbq-watch-csv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut logger = CsvLogger::new(true, dir.clone(), 10);
        let at = AppInstant::from_secs_f64(0.0);
        let temps = [Some(65.0), None, Some(21.5), Some(120.0)];
        logger.write_row(at, Local::now(), &temps, Some(80), "");
        logger.flush();
        let name = logger.file_name().unwrap();
        let contents = std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(contents.starts_with("timestamp,elapsed_s,p1_c"));
        assert_eq!(contents.lines().count(), 2);
        assert!(contents.contains("80,"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
