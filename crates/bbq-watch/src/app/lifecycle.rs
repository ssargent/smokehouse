//! Shutdown and terminal-side effects.

use super::*;

impl App {
    /// Write the terminal bell if one is pending and enabled.
    pub(crate) fn emit_bell(&mut self) {
        if !self.bell_pending {
            return;
        }
        self.bell_pending = false;
        if self.settings.alarms.bell {
            let _ = ratatui::crossterm::execute!(
                std::io::stdout(),
                ratatui::crossterm::style::Print("\u{7}")
            );
        }
    }

    /// Flush the log, save settings and stop the background tasks.
    pub(crate) async fn cleanup(&mut self) {
        self.logger.flush();
        self.flush_settings(true);
        self.caffeinate.stop();
        let _ = self.device_tx.send(DeviceCommand::Shutdown).await;
        self.notifier.shutdown();
    }
}
