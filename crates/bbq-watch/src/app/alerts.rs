//! Alert view syncing, acknowledgement and status speech.

use super::*;

impl App {
    pub(crate) async fn sync_alerts_view(&mut self, now: AppInstant, snapshot: &Snapshot) {
        let serve_at = self.serve_wall.map(|wall| wall.format("%H:%M").to_string());
        let mut shared = self.shared.lock().await;
        shared.sync(
            &self.engine,
            snapshot,
            self.unit(),
            self.battery,
            serve_at,
            now,
        );
        let mut rows: Vec<AlertRow> = shared
            .entries()
            .iter()
            .map(|(key, info): (&AlertKey, &ActiveInfo)| AlertRow {
                key: *key,
                priority: info.priority,
                since: info.since,
                last_spoken: info.last_spoken,
                acked: info.acked,
                snoozed_until: info.snoozed_until,
            })
            .collect();
        rows.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| b.since.secs().total_cmp(&a.since.secs()))
        });
        let banner = rows.iter().find(|row| !row.acked).map(|row| Banner {
            text: shared.build_text(&row.key),
            priority: row.priority,
        });
        drop(shared);
        self.alerts_view.banner = banner;
        self.alerts_view.rows = rows;
    }

    pub(crate) fn maybe_speak_status(&mut self, now: AppInstant) {
        let every = self.settings.speech.status_every_min;
        if every == 0 {
            self.next_status_at = None;
            return;
        }
        let due = self
            .next_status_at
            .is_none_or(|at| now.since(at) >= f64::from(every) * 60.0);
        if due && self.connected() {
            self.next_status_at = Some(now);
            self.notifier.status();
        }
    }

    pub(crate) fn ack_all(&mut self) {
        let snapshot = self.snapshot();
        self.engine.ack_all(&snapshot);
    }

    pub(crate) fn snooze_all(&mut self) {
        let minutes = f64::from(self.settings.alarms.snooze_min);
        self.engine.snooze(self.now, minutes);
    }
}
