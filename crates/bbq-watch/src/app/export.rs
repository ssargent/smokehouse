//! History export to a CSV file.

use super::*;

impl App {
    pub(crate) fn export_history(&mut self) {
        let dir = self.settings.csv_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            self.log_event(
                EventIcon::Warning,
                String::from("Could not create export directory"),
            );
            return;
        }
        let wall = self.clock.wall(self.now);
        let path = dir.join(format!("history-{}.csv", wall.format("%Y-%m-%d-%H%M")));
        match self.write_history_csv(&path) {
            Ok(()) => {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.log_event(EventIcon::Info, format!("Exported history to {name}"));
            }
            Err(err) => {
                self.log_event(EventIcon::Warning, format!("Export failed: {err}"));
            }
        }
    }

    /// Write the merged history to `path`, with the same columns as the logger.
    fn write_history_csv(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        use std::collections::BTreeMap;
        let file = std::fs::File::create(path)?;
        let mut writer = csv::Writer::from_writer(file);
        let mut table: BTreeMap<i64, [Option<f32>; PROBE_COUNT]> = BTreeMap::new();
        for index in 0..PROBE_COUNT {
            for (at, temp) in self.probes[index].history.samples() {
                let entry = table.entry(at.secs() as i64).or_insert([None; PROBE_COUNT]);
                entry[index] = *temp;
            }
        }
        writer.write_record(crate::logger::HEADER)?;
        let started = self.clock.wall(AppInstant::ZERO);
        for (elapsed, temps) in table {
            let at = started + chrono::Duration::seconds(elapsed);
            let mut record: Vec<String> = Vec::with_capacity(crate::logger::HEADER.len());
            record.push(at.format("%Y-%m-%d %H:%M:%S").to_string());
            record.push(elapsed.to_string());
            record.extend(
                temps
                    .iter()
                    .map(|t| t.map_or_else(String::new, |c| format!("{c:.1}"))),
            );
            record.extend(temps.iter().map(|t| {
                t.map_or_else(String::new, |c| format!("{:.1}", celsius_to_fahrenheit(c)))
            }));
            record.push(String::new());
            record.push(String::new());
            writer.write_record(&record)?;
        }
        writer.flush()?;
        Ok(())
    }
}
