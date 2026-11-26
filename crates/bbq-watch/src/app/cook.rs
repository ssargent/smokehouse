//! Per-tick cook analysis: probe updates, lid detection and the summary.

use super::*;
use crate::analysis::{EtaInput, StallTransition, compute_eta};
use crate::config::CookSettings;

impl App {
    /// One tick of per-probe history, stall tracking and ETA.
    pub(crate) fn update_probes(
        &mut self,
        now: AppInstant,
        cook: &CookSettings,
        new_events: &mut Vec<(EventIcon, String)>,
    ) {
        for index in 0..PROBE_COUNT {
            let cfg = self.settings.probe(index);
            let probe = &mut self.probes[index];
            probe.history.push(now, probe.temp_c);
            if probe.start_temp_c.is_none() {
                probe.start_temp_c = probe.history.current();
            }
            let rate = probe.history.rate_c_per_h(now, cook.rate_window_min);
            probe.rate_c_h = rate;
            probe.deviation_c = probe.history.max_deviation_c(now, 10.0);
            let transition = probe.stall.update(now, probe.temp_c, rate, cook);
            probe.stalled = probe.stall.is_stalled();
            match transition {
                StallTransition::Started => {
                    new_events.push((EventIcon::Warning, format!("{} stalled", cfg.label)));
                }
                StallTransition::Ended => {
                    new_events.push((EventIcon::Milestone, format!("{} stall ended", cfg.label)));
                }
                StallTransition::None => {}
            }
            if !self.settings.is_pit(index) {
                if let Some(temp) = probe.temp_c {
                    let fahrenheit = celsius_to_fahrenheit(temp);
                    let milestone = (fahrenheit / 20.0).floor() * 20.0;
                    if milestone >= 100.0 && milestone as i32 > probe.last_milestone_f {
                        probe.last_milestone_f = milestone as i32;
                        new_events.push((
                            EventIcon::Milestone,
                            format!("{} passed {milestone:.0}°F", cfg.label),
                        ));
                    }
                }
            } else if let Some(range) = cfg.range_c
                && probe.temp_c.is_some_and(|temp| temp >= range[0])
                && !probe.reached_range_min
            {
                probe.reached_range_min = true;
                new_events.push((EventIcon::Milestone, String::from("Pit reached range")));
            }
            probe.eta = cfg.target_c.map(|target_c| {
                compute_eta(
                    &EtaInput {
                        target_c,
                        current_c: probe.temp_c,
                        rate_c_h: rate,
                        stalled: probe.stalled,
                        stall_started_at: probe.stall.started_at(),
                        wrapped_at: probe.wrapped_at,
                        history: &probe.history,
                    },
                    now,
                    cook,
                )
            });
        }
    }

    pub(crate) fn detect_lid(
        &mut self,
        now: AppInstant,
        new_events: &mut Vec<(EventIcon, String)>,
    ) {
        let pit_index = (0..PROBE_COUNT).find(|index| self.is_pit(*index));
        let Some(index) = pit_index else {
            return;
        };
        let current = self.probes[index].temp_c;
        let past = self.probes[index]
            .history
            .value_near(AppInstant::from_secs_f64(
                now.secs() - LID_WINDOW_MIN * 60.0,
            ));
        if let (Some(now_temp), Some(past_temp)) = (current, past) {
            let drop = celsius_to_fahrenheit(past_temp - now_temp);
            if drop > 25.0 && !self.lid_open {
                self.lid_open = true;
                self.pit_reference_c = Some(past_temp);
                new_events.push((EventIcon::Warning, String::from("Lid open")));
                self.frozen_etas = Some(std::array::from_fn(|i| {
                    self.probes[i].eta.and_then(|eta| eta.seconds)
                }));
            } else if self.lid_open
                && let Some(reference) = self.pit_reference_c
                && celsius_to_fahrenheit(reference - now_temp).abs() <= 10.0
            {
                self.lid_open = false;
                self.pit_reference_c = None;
                self.frozen_etas = None;
                new_events.push((EventIcon::Milestone, String::from("Pit recovered")));
            }
        }
    }

    pub(crate) fn compute_summary(&mut self, now: AppInstant) {
        let mut all_done: Option<f64> = None;
        for index in 0..PROBE_COUNT {
            if self.is_pit(index) {
                continue;
            }
            let seconds = if self.settings.cook.pause_eta_on_lid_open && self.lid_open {
                self.frozen_etas
                    .and_then(|frozen| frozen[index])
                    .or_else(|| self.probes[index].eta.and_then(|eta| eta.seconds))
            } else {
                self.probes[index].eta.and_then(|eta| eta.seconds)
            };
            if let Some(seconds) = seconds {
                all_done = Some(all_done.map_or(seconds, |current: f64| current.max(seconds)));
            }
        }
        self.all_done_seconds = all_done;
        self.serve_wall = all_done.map(|seconds| {
            let serve_at = AppInstant::from_secs_f64(
                now.secs() + seconds + self.settings.cook.rest_min * 60.0,
            );
            self.clock.wall(serve_at)
        });
        self.serve_late = match (
            self.serve_wall,
            parse_hhmm(&self.settings.cook.want_to_eat_at),
        ) {
            (Some(serve), Some((want_h, want_m))) => {
                let start = self.clock.wall(AppInstant::ZERO);
                serve_deadline(start, want_h, want_m).is_some_and(|want| serve > want)
            }
            _ => false,
        };
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let probes = (0..PROBE_COUNT)
            .map(|index| {
                let cfg = self.settings.probe(index);
                let is_pit = self.settings.is_pit(index);
                ProbeSnapshot {
                    id: ProbeId::from_index(index),
                    label: cfg.label.clone(),
                    temp_c: self.probes[index].temp_c,
                    target_c: cfg.target_c,
                    alert_high_c: cfg.alert_high_c,
                    alert_low_c: cfg.alert_low_c,
                    pre_alert_c: cfg.pre_alert_c,
                    is_pit,
                    range_c: cfg.range_c,
                    stalled: self.probes[index].stalled,
                }
            })
            .collect();
        Snapshot {
            probes,
            battery: self.battery,
            last_reading_at: self.last_reading_at,
            serve_late: self.serve_late,
            alert_on_stall: self.settings.alarms.alert_on_stall,
        }
    }

    /// Push an event, mirroring it into the CSV log.
    pub(crate) fn log_event(&mut self, icon: EventIcon, message: String) {
        let now = self.now;
        let wall = self.clock.wall(now);
        self.events.push(Event {
            wall,
            icon,
            message: message.clone(),
        });
        let temps = self.temps_c();
        let battery = self.battery;
        self.logger.write_row(now, wall, &temps, battery, &message);
    }
}
