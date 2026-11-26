//! Key input for every tab, plus the quit prompt.

use super::*;

impl App {
    pub(crate) fn on_input(&mut self, event: CrosstermEvent) {
        let CrosstermEvent::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }
        if self.show_help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
            ) {
                self.show_help = false;
            }
            return;
        }
        if self.quit_prompt {
            match key.code {
                KeyCode::Char('y' | 'Y') => self.should_quit = true,
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.quit_prompt = false,
                _ => {}
            }
            return;
        }
        if self.tab == crate::ui::Tab::Settings && self.settings_ui.overlay_active() {
            ui::settings::handle_key(self, key);
            return;
        }

        // Keys that are global on every tab, including Settings.
        match key.code {
            KeyCode::Char('q') => {
                self.request_quit();
                return;
            }
            KeyCode::Char('x') => {
                self.ack_all();
                return;
            }
            KeyCode::Char('z') => {
                self.snooze_all();
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                return;
            }
            KeyCode::Char('1') => {
                self.tab = Tab::Dashboard;
                return;
            }
            KeyCode::Char('2') => {
                self.tab = Tab::Table;
                return;
            }
            KeyCode::Char('3') => {
                self.tab = Tab::Alarms;
                return;
            }
            KeyCode::Char('4') => {
                self.tab = Tab::Log;
                return;
            }
            KeyCode::Char('5') => {
                self.tab = Tab::Settings;
                return;
            }
            _ => {}
        }

        if self.tab == Tab::Settings {
            ui::settings::handle_key(self, key);
            return;
        }

        match key.code {
            KeyCode::Left => self.tab = self.tab.prev(),
            KeyCode::Right => self.tab = self.tab.next(),
            _ => {}
        }

        if self.demo {
            self.handle_demo_keys(key);
        }
        match self.tab {
            Tab::Dashboard => self.handle_dashboard_key(key),
            Tab::Table => self.handle_table_key(key),
            Tab::Alarms => self.handle_alarms_key(key),
            Tab::Log => self.handle_log_key(key),
            Tab::Settings => {}
        }
    }

    fn handle_demo_keys(&mut self, key: KeyEvent) {
        let slot = match key.code {
            KeyCode::Char('!') => 0,
            KeyCode::Char('@') => 1,
            KeyCode::Char('#') => 2,
            KeyCode::Char('$') => 3,
            _ => return,
        };
        self.demo_unplugged[slot] = !self.demo_unplugged[slot];
        let state = if self.demo_unplugged[slot] {
            "unplugged"
        } else {
            "plugged back in"
        };
        let label = self.probe_label(slot);
        self.log_event(EventIcon::Info, format!("{label} {state}"));
    }

    fn handle_dashboard_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::BackTab => {
                self.selected_card = self.selected_card.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected_card = (self.selected_card + 1).min(PROBE_COUNT - 1);
            }
            KeyCode::Char('w') if !self.is_pit(self.selected_card) => {
                self.probes[self.selected_card].wrapped_at = Some(self.now);
                let label = self.probe_label(self.selected_card);
                self.log_event(EventIcon::Info, format!("{label} wrapped"));
            }
            _ => {}
        }
    }

    fn handle_table_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.table_scroll = self.table_scroll.saturating_sub(1),
            KeyCode::Down => self.table_scroll += 1,
            KeyCode::PageUp => self.table_scroll = self.table_scroll.saturating_sub(10),
            KeyCode::PageDown => self.table_scroll += 10,
            KeyCode::Home => self.table_scroll = 0,
            KeyCode::End => self.table_scroll = usize::MAX,
            KeyCode::Char('i') => {
                self.settings.display.table_interval_min =
                    match self.settings.display.table_interval_min {
                        1 => 5,
                        5 => 15,
                        _ => 1,
                    };
                self.table_scroll = 0;
            }
            KeyCode::Char('e') => self.export_history(),
            _ => {}
        }
    }

    fn handle_alarms_key(&mut self, key: KeyEvent) {
        let count = self.alerts_view.rows.len();
        match key.code {
            KeyCode::Up => self.alarms_selected = self.alarms_selected.saturating_sub(1),
            KeyCode::Down => {
                if count > 0 {
                    self.alarms_selected = (self.alarms_selected + 1).min(count - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(row) = self.alerts_view.rows.get(self.alarms_selected) {
                    let key = row.key;
                    let snapshot = self.snapshot();
                    self.engine.ack(key, &snapshot);
                }
            }
            KeyCode::Char('t') => self.notifier.test(),
            _ => {}
        }
    }

    fn handle_log_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.log_scroll = self.log_scroll.saturating_sub(1),
            KeyCode::Down => self.log_scroll += 1,
            KeyCode::PageUp => self.log_scroll = self.log_scroll.saturating_sub(10),
            KeyCode::PageDown => self.log_scroll += 10,
            KeyCode::Home => self.log_scroll = 0,
            KeyCode::End => self.log_scroll = usize::MAX,
            _ => {}
        }
    }

    fn request_quit(&mut self) {
        if self.connected() {
            self.quit_prompt = true;
        } else {
            self.should_quit = true;
        }
    }
}
