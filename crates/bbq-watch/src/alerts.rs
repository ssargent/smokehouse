//! Alert engine: level-triggered conditions with hysteresis, per-key cooldowns,
//! acknowledgement and snooze. Pure and synchronous; the app owns all I/O.

use std::collections::{HashMap, HashSet};

use crate::clock::AppInstant;
use crate::model::{ProbeId, Unit, f_delta_to_c};

/// A kind of alert. Together with a probe id it forms a stable [`AlertKey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AlertKind {
    /// Temperature reached the probe target.
    TargetReached,
    /// Temperature is approaching the target.
    PreAlert,
    /// Temperature rose past the high threshold.
    AboveHigh,
    /// Temperature fell below the low threshold.
    BelowLow,
    /// A probe stopped reading.
    ProbeUnplugged,
    /// No readings arrived for a while.
    BluetoothLost,
    /// A stall started.
    StallStarted,
    /// A stall ended.
    StallEnded,
    /// Battery is low.
    BatteryLow,
    /// The cook will finish after the requested serve time.
    ServeLate,
}

impl AlertKind {
    /// All kinds, in display order.
    pub const ALL: [Self; 10] = [
        Self::TargetReached,
        Self::PreAlert,
        Self::AboveHigh,
        Self::BelowLow,
        Self::ProbeUnplugged,
        Self::BluetoothLost,
        Self::StallStarted,
        Self::StallEnded,
        Self::BatteryLow,
        Self::ServeLate,
    ];

    /// Human-readable name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::TargetReached => "Target reached",
            Self::PreAlert => "Pre-alert",
            Self::AboveHigh => "Above high",
            Self::BelowLow => "Below low",
            Self::ProbeUnplugged => "Probe unplugged",
            Self::BluetoothLost => "Bluetooth lost",
            Self::StallStarted => "Stall started",
            Self::StallEnded => "Stall ended",
            Self::BatteryLow => "Battery low",
            Self::ServeLate => "Serve late",
        }
    }

    /// Delivery priority.
    #[must_use]
    pub fn priority(self) -> Priority {
        match self {
            Self::TargetReached
            | Self::AboveHigh
            | Self::BelowLow
            | Self::ProbeUnplugged
            | Self::BluetoothLost => Priority::Urgent,
            Self::PreAlert
            | Self::StallStarted
            | Self::StallEnded
            | Self::BatteryLow
            | Self::ServeLate => Priority::Normal,
        }
    }

    /// One-shots are delivered once and never repeat.
    #[must_use]
    pub fn is_one_shot(self) -> bool {
        matches!(self, Self::StallStarted | Self::StallEnded)
    }

    /// `+1` when the alarming direction is up, `-1` down, `0` otherwise.
    #[must_use]
    fn direction(self) -> i8 {
        match self {
            Self::TargetReached | Self::PreAlert | Self::AboveHigh => 1,
            Self::BelowLow => -1,
            _ => 0,
        }
    }
}

/// Alert identity: an optional probe plus the kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlertKey {
    /// The probe the alert is about, if any (None = device-wide).
    pub probe: Option<ProbeId>,
    /// The alert kind.
    pub kind: AlertKind,
}

impl AlertKey {
    /// Build a key.
    #[must_use]
    pub fn new(probe: Option<ProbeId>, kind: AlertKind) -> Self {
        Self { probe, kind }
    }
}

/// Delivery priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    /// Interrupts everything.
    Urgent,
    /// Queued behind urgent items.
    Normal,
}

/// One alert activation to deliver.
#[derive(Clone, Debug)]
pub struct Delivery {
    /// Which alert.
    pub key: AlertKey,
    /// Delivery priority.
    pub priority: Priority,
}

/// What one evaluation produced: alerts to deliver and alerts that cleared.
#[derive(Debug, Default)]
pub struct Evaluation {
    /// Alerts to deliver this pass.
    pub deliveries: Vec<Delivery>,
    /// Alerts that cleared during this pass.
    pub cleared: Vec<(AlertKey, AppInstant)>,
}

/// A probe's current state as the engine sees it.
#[derive(Clone, Debug)]
pub struct ProbeSnapshot {
    /// Probe id.
    pub id: ProbeId,
    /// Current label.
    pub label: String,
    /// Current temperature in °C.
    pub temp_c: Option<f32>,
    /// Target temperature in °C.
    pub target_c: Option<f32>,
    /// High threshold in °C.
    pub alert_high_c: Option<f32>,
    /// Low threshold in °C.
    pub alert_low_c: Option<f32>,
    /// Pre-alert distance below the target in °C.
    pub pre_alert_c: Option<f32>,
    /// Whether this is the pit probe.
    pub is_pit: bool,
    /// Pit range `[min, max]` in °C.
    pub range_c: Option<[f32; 2]>,
    /// Whether the probe is currently stalled.
    pub stalled: bool,
}

/// Everything the engine needs for one evaluation.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Per-probe state.
    pub probes: Vec<ProbeSnapshot>,
    /// Battery percentage.
    pub battery: Option<u8>,
    /// Instant of the most recent reading.
    pub last_reading_at: Option<AppInstant>,
    /// Whether the serve time is later than the requested time.
    pub serve_late: bool,
    /// Whether stall alerts are enabled.
    pub alert_on_stall: bool,
}

/// A condition sampled for one key: `fire` and `clear` are both false inside the
/// hysteresis band, where the previous state is kept.
#[derive(Clone, Copy, Debug)]
struct Condition {
    key: AlertKey,
    fire: bool,
    clear: bool,
}

/// Engine state for one active alert.
#[derive(Clone, Debug)]
pub struct Active {
    /// When the alert became active.
    pub since: AppInstant,
    /// When it was last delivered.
    pub last_delivery: AppInstant,
    /// Number of deliveries so far.
    pub deliveries: u32,
    /// Whether it has been acknowledged.
    pub acked: bool,
    /// Temperature when acknowledged (for the 5 °F re-arm rule).
    ack_temp_c: Option<f32>,
}

/// The pure alert engine.
#[derive(Debug)]
pub struct AlertEngine {
    active: HashMap<AlertKey, Active>,
    cleared_recent: Vec<(AlertKey, AppInstant)>,
    snooze_until: Option<AppInstant>,
    unplugged_since: HashMap<ProbeId, AppInstant>,
    had_reading: HashSet<ProbeId>,
    prev_stalled: HashMap<ProbeId, bool>,
    armed_low: HashSet<ProbeId>,
    repeat_s: u32,
    bt_lost_after_min: u32,
    unplugged_after_s: u32,
}

impl AlertEngine {
    /// Create an engine with the given timing limits.
    #[must_use]
    pub fn new(repeat_s: u32, bt_lost_after_min: u32, unplugged_after_s: u32) -> Self {
        Self {
            active: HashMap::new(),
            cleared_recent: Vec::new(),
            snooze_until: None,
            unplugged_since: HashMap::new(),
            had_reading: HashSet::new(),
            prev_stalled: HashMap::new(),
            armed_low: HashSet::new(),
            repeat_s,
            bt_lost_after_min,
            unplugged_after_s,
        }
    }

    /// Update the timing limits from settings.
    pub fn set_limits(&mut self, repeat_s: u32, bt_lost_after_min: u32, unplugged_after_s: u32) {
        self.repeat_s = repeat_s;
        self.bt_lost_after_min = bt_lost_after_min;
        self.unplugged_after_s = unplugged_after_s;
    }

    /// Active alerts, keyed.
    #[must_use]
    pub fn active(&self) -> &HashMap<AlertKey, Active> {
        &self.active
    }

    /// When the current snooze ends, if any.
    #[must_use]
    pub fn snooze_until(&self) -> Option<AppInstant> {
        self.snooze_until
    }

    /// Alerts cleared within the last hour.
    #[must_use]
    pub fn cleared_recent(&self, now: AppInstant) -> Vec<(AlertKey, AppInstant)> {
        self.cleared_recent
            .iter()
            .filter(|(_, at)| now.since(*at) <= 3600.0)
            .copied()
            .collect()
    }

    /// Snooze all alerts for `minutes`.
    pub fn snooze(&mut self, now: AppInstant, minutes: f64) {
        self.snooze_until = Some(now.plus_minutes(minutes));
    }

    /// Acknowledge every active alert.
    pub fn ack_all(&mut self, snapshot: &Snapshot) {
        let keys: Vec<AlertKey> = self.active.keys().copied().collect();
        for key in keys {
            self.ack(key, snapshot);
        }
    }

    /// Acknowledge one alert.
    pub fn ack(&mut self, key: AlertKey, snapshot: &Snapshot) {
        let temp = probe_temp(key.probe, snapshot);
        if let Some(active) = self.active.get_mut(&key) {
            active.acked = true;
            active.ack_temp_c = temp;
        }
    }

    /// Feed a new snapshot; returns the deliveries and clears for this pass.
    pub fn evaluate(&mut self, snapshot: &Snapshot, now: AppInstant) -> Evaluation {
        let mut deliveries = Vec::new();
        let mut cleared = Vec::new();
        let margin = f_delta_to_c(2.0);
        let mut conditions: Vec<Condition> = Vec::new();

        for probe in &snapshot.probes {
            let id = Some(probe.id);
            let temp = probe.temp_c;

            if let Some(target) = probe.target_c {
                let (fire, clear) = upper(temp, target, margin);
                conditions.push(Condition {
                    key: AlertKey::new(id, AlertKind::TargetReached),
                    fire,
                    clear,
                });
                let pre = probe.pre_alert_c;
                let (pre_fire, pre_clear) = pre_alert_state(temp, target, pre, margin);
                conditions.push(Condition {
                    key: AlertKey::new(id, AlertKind::PreAlert),
                    fire: pre_fire,
                    clear: pre_clear || fire,
                });
            }

            let high = probe
                .alert_high_c
                .or_else(|| probe.range_c.map(|range| range[1]));
            if let Some(threshold) = high {
                let (fire, clear) = upper(temp, threshold, margin);
                conditions.push(Condition {
                    key: AlertKey::new(id, AlertKind::AboveHigh),
                    fire,
                    clear,
                });
            }

            let low = probe
                .alert_low_c
                .or_else(|| probe.range_c.map(|range| range[0]));
            if let Some(threshold) = low {
                if temp.is_some_and(|t| t > threshold) {
                    self.armed_low.insert(probe.id);
                }
                let armed = self.armed_low.contains(&probe.id);
                let (fire, clear) = lower(temp, threshold, margin);
                conditions.push(Condition {
                    key: AlertKey::new(id, AlertKind::BelowLow),
                    fire: armed && fire,
                    clear,
                });
            }

            // Unplugged tracking.
            match temp {
                Some(_) => {
                    self.had_reading.insert(probe.id);
                    self.unplugged_since.remove(&probe.id);
                }
                None => {
                    self.unplugged_since.entry(probe.id).or_insert(now);
                }
            }
            let unplugged = self.had_reading.contains(&probe.id)
                && self
                    .unplugged_since
                    .get(&probe.id)
                    .is_some_and(|since| now.since(*since) >= f64::from(self.unplugged_after_s));
            conditions.push(Condition {
                key: AlertKey::new(id, AlertKind::ProbeUnplugged),
                fire: unplugged,
                clear: temp.is_some(),
            });

            // Stall one-shots.
            let previous = self.prev_stalled.get(&probe.id).copied().unwrap_or(false);
            if snapshot.alert_on_stall && probe.stalled && !previous {
                deliveries.push(make_delivery(
                    AlertKey::new(id, AlertKind::StallStarted),
                    now,
                    snapshot,
                ));
            }
            if snapshot.alert_on_stall && !probe.stalled && previous {
                deliveries.push(make_delivery(
                    AlertKey::new(id, AlertKind::StallEnded),
                    now,
                    snapshot,
                ));
            }
            self.prev_stalled.insert(probe.id, probe.stalled);
        }

        // Bluetooth lost.
        let lost = snapshot
            .last_reading_at
            .is_some_and(|at| now.since(at) >= f64::from(self.bt_lost_after_min) * 60.0);
        conditions.push(Condition {
            key: AlertKey::new(None, AlertKind::BluetoothLost),
            fire: lost,
            clear: !lost && snapshot.last_reading_at.is_some(),
        });

        // Battery.
        if let Some(percent) = snapshot.battery {
            let fire = percent < 20;
            let clear = percent >= 25;
            if fire || clear {
                conditions.push(Condition {
                    key: AlertKey::new(None, AlertKind::BatteryLow),
                    fire,
                    clear,
                });
            }
        }

        // Serve late.
        conditions.push(Condition {
            key: AlertKey::new(None, AlertKind::ServeLate),
            fire: snapshot.serve_late,
            clear: !snapshot.serve_late,
        });

        let snoozed = self.snooze_until.is_some_and(|until| now < until);
        for condition in conditions {
            if let Some(delivery) = self.apply(condition, now, snapshot, snoozed, &mut cleared) {
                deliveries.push(delivery);
            }
        }

        self.cleared_recent
            .retain(|(_, at)| now.since(*at) <= 3600.0);

        Evaluation {
            deliveries,
            cleared,
        }
    }

    fn apply(
        &mut self,
        condition: Condition,
        now: AppInstant,
        snapshot: &Snapshot,
        snoozed: bool,
        cleared: &mut Vec<(AlertKey, AppInstant)>,
    ) -> Option<Delivery> {
        let key = condition.key;

        if condition.clear {
            if self.active.remove(&key).is_some() {
                self.cleared_recent.push((key, now));
                cleared.push((key, now));
            }
            return None;
        }
        if !condition.fire {
            return None;
        }

        let temp = probe_temp(key.probe, snapshot);
        match self.active.get_mut(&key) {
            None => {
                self.active.insert(
                    key,
                    Active {
                        since: now,
                        last_delivery: now,
                        deliveries: 1,
                        acked: false,
                        ack_temp_c: None,
                    },
                );
                Some(make_delivery(key, now, snapshot))
            }
            Some(active) => {
                if let (Some(ack_temp), Some(current)) = (active.ack_temp_c, temp) {
                    let direction = f32::from(key.kind.direction());
                    if (current - ack_temp) * direction >= f_delta_to_c(5.0) {
                        active.acked = false;
                        active.ack_temp_c = None;
                        active.last_delivery = now;
                        active.deliveries += 1;
                        return Some(make_delivery(key, now, snapshot));
                    }
                }
                if active.acked || snoozed {
                    return None;
                }
                if now.since(active.last_delivery) >= repeat_gap(self.repeat_s, active.deliveries) {
                    active.last_delivery = now;
                    active.deliveries += 1;
                    return Some(make_delivery(key, now, snapshot));
                }
                None
            }
        }
    }
}

/// Current temperature for the probe in a key, if any.
fn probe_temp(id: Option<ProbeId>, snapshot: &Snapshot) -> Option<f32> {
    let id = id?;
    snapshot
        .probes
        .iter()
        .find(|probe| probe.id == id)
        .and_then(|probe| probe.temp_c)
}

/// Seconds before the next repeat, given how many deliveries have happened.
fn repeat_gap(repeat_s: u32, deliveries: u32) -> f64 {
    match deliveries {
        0 | 1 => f64::from(repeat_s),
        2 => f64::from(2 * repeat_s),
        _ => (3.33 * f64::from(repeat_s)).max(300.0),
    }
}

fn upper(temp: Option<f32>, threshold: f32, margin: f32) -> (bool, bool) {
    match temp {
        Some(value) => (value >= threshold, value < threshold - margin),
        None => (false, false),
    }
}

fn lower(temp: Option<f32>, threshold: f32, margin: f32) -> (bool, bool) {
    match temp {
        Some(value) => (value < threshold, value > threshold + margin),
        None => (false, false),
    }
}

fn pre_alert_state(
    temp: Option<f32>,
    target: f32,
    pre_alert: Option<f32>,
    margin: f32,
) -> (bool, bool) {
    let Some(distance) = pre_alert else {
        return (false, false);
    };
    match temp {
        Some(value) => {
            let threshold = target - distance;
            (
                value >= threshold && value < target,
                value < threshold - margin || value >= target,
            )
        }
        None => (false, false),
    }
}

fn make_delivery(key: AlertKey, _now: AppInstant, _snapshot: &Snapshot) -> Delivery {
    Delivery {
        key,
        priority: key.kind.priority(),
    }
}

/// Snapshot of one active alert, as seen outside the engine.
#[derive(Clone, Debug)]
pub struct ActiveInfo {
    /// Priority.
    pub priority: Priority,
    /// When it became active.
    pub since: AppInstant,
    /// Whether it has been acknowledged.
    pub acked: bool,
    /// When it was last spoken.
    pub last_spoken: Option<AppInstant>,
    /// Snooze expiry, if snoozed.
    pub snoozed_until: Option<AppInstant>,
}

/// The small shared snapshot the speech worker reads: which alerts are active
/// and unacknowledged, plus the live probe values needed to phrase them.
#[derive(Debug, Default)]
pub struct ActiveAlerts {
    entries: HashMap<AlertKey, ActiveInfo>,
    snapshot: Snapshot,
    unit: Unit,
    battery: Option<u8>,
    serve_at: Option<String>,
}

impl ActiveAlerts {
    /// Rebuild from the engine and current snapshot, preserving `last_spoken`.
    pub fn sync(
        &mut self,
        engine: &AlertEngine,
        snapshot: &Snapshot,
        unit: Unit,
        battery: Option<u8>,
        serve_at: Option<String>,
        now: AppInstant,
    ) {
        self.unit = unit;
        self.battery = battery;
        self.serve_at = serve_at;
        self.snapshot = snapshot.clone();

        let snooze = engine.snooze_until().filter(|until| now < *until);
        let mut entries = HashMap::new();
        for (key, active) in engine.active() {
            let last_spoken = self.entries.get(key).and_then(|info| info.last_spoken);
            entries.insert(
                *key,
                ActiveInfo {
                    priority: key.kind.priority(),
                    since: active.since,
                    acked: active.acked,
                    last_spoken,
                    snoozed_until: snooze,
                },
            );
        }
        self.entries = entries;
    }

    /// Whether an alert is active and not acknowledged.
    #[must_use]
    pub fn is_active_unacked(&self, key: &AlertKey) -> bool {
        self.entries.get(key).is_some_and(|info| !info.acked)
    }

    /// Record that an alert was spoken at `at`.
    pub fn mark_spoken(&mut self, key: &AlertKey, at: AppInstant) {
        if let Some(info) = self.entries.get_mut(key) {
            info.last_spoken = Some(at);
        }
    }

    /// Active alerts, keyed.
    #[must_use]
    pub fn entries(&self) -> &HashMap<AlertKey, ActiveInfo> {
        &self.entries
    }

    /// Live probe info.
    #[must_use]
    pub fn probes(&self) -> &[ProbeSnapshot] {
        &self.snapshot.probes
    }

    /// Build the spoken phrase for an alert from current numbers.
    #[must_use]
    pub fn build_text(&self, key: &AlertKey) -> String {
        let kind = key.kind;
        let probe = key
            .probe
            .and_then(|id| self.snapshot.probes.iter().find(|p| p.id == id));
        let label = probe.map_or_else(String::new, |p| p.label.clone());
        let temp = probe.and_then(|p| p.temp_c).map(|c| self.unit.from_c(c));
        let degrees = |value: f32| format!("{} degrees", value.round());
        match kind {
            AlertKind::TargetReached => match temp {
                Some(value) => format!("{label} is at {}. It's ready.", degrees(value)),
                None => format!("{label} is ready."),
            },
            AlertKind::PreAlert => {
                let from_target = probe
                    .and_then(|p| p.target_c)
                    .zip(probe.and_then(|p| p.temp_c))
                    .map(|(target, current)| self.unit.from_c(target - current));
                match (temp, from_target) {
                    (Some(value), Some(delta)) => format!(
                        "{label} is at {}, {} degrees from target.",
                        degrees(value),
                        delta.round()
                    ),
                    _ => format!("{label} is near its target."),
                }
            }
            AlertKind::AboveHigh => match temp {
                Some(value) => format!("Warning. {label} has risen to {}.", degrees(value)),
                None => format!("Warning. {label} is too hot."),
            },
            AlertKind::BelowLow => match temp {
                Some(value) => format!("Warning. {label} has dropped to {}.", degrees(value)),
                None => format!("Warning. {label} is too cool."),
            },
            AlertKind::ProbeUnplugged => format!("{label} probe is disconnected."),
            AlertKind::BluetoothLost => String::from("Lost connection to the thermometer."),
            AlertKind::StallStarted => match temp {
                Some(value) => format!("{label} has stalled at {}.", degrees(value)),
                None => format!("{label} has stalled."),
            },
            AlertKind::StallEnded => match temp {
                Some(value) => format!("{label} is moving again at {}.", degrees(value)),
                None => format!("{label} is moving again."),
            },
            AlertKind::BatteryLow => match self.battery {
                Some(percent) => format!("Thermometer battery is at {percent} percent."),
                None => String::from("Thermometer battery is low."),
            },
            AlertKind::ServeLate => match &self.serve_at {
                Some(time) => format!(
                    "Heads up. The cook is now estimated to finish after {}.",
                    time.replace(':', " ")
                ),
                None => String::from("Heads up. The cook is running late."),
            },
        }
    }

    /// Build the periodic spoken status summary.
    #[must_use]
    pub fn build_status(&self) -> String {
        let mut parts = Vec::new();
        let mut pit = None;
        for probe in &self.snapshot.probes {
            if let Some(temp) = probe.temp_c {
                let value = self.unit.from_c(temp).round();
                if probe.is_pit {
                    pit = Some(value);
                } else {
                    parts.push(format!("{} {value}", probe.label));
                }
            }
        }
        let mut text = parts.join(", ");
        if let Some(value) = pit {
            if text.is_empty() {
                text = format!("Pit steady at {value}");
            } else {
                text.push_str(&format!(". Pit steady at {value}"));
            }
        }
        if text.is_empty() {
            String::from("No probes are reading.")
        } else {
            text.push('.');
            text
        }
    }
}

/// A short, log-style phrase for an alert (distinct from the spoken text).
#[must_use]
pub fn short_text(key: &AlertKey, snapshot: &Snapshot) -> String {
    let probe = key
        .probe
        .and_then(|id| snapshot.probes.iter().find(|probe| probe.id == id));
    let label = probe.map_or_else(String::new, |probe| probe.label.clone());
    match key.kind {
        AlertKind::TargetReached => format!("{label} reached target"),
        AlertKind::PreAlert => format!("{label} approaching target"),
        AlertKind::AboveHigh => format!("{label} above high"),
        AlertKind::BelowLow => format!("{label} below low"),
        AlertKind::ProbeUnplugged => format!("{label} unplugged"),
        AlertKind::BluetoothLost => String::from("Bluetooth lost"),
        AlertKind::StallStarted => format!("{label} stalled"),
        AlertKind::StallEnded => format!("{label} stall ended"),
        AlertKind::BatteryLow => String::from("Battery low"),
        AlertKind::ServeLate => String::from("Serve time is late"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::fahrenheit_to_celsius;

    fn probe(id: u8, temp_f: f32, target_f: f32) -> ProbeSnapshot {
        ProbeSnapshot {
            id: ProbeId::new(id).unwrap(),
            label: format!("P{id}"),
            temp_c: Some(fahrenheit_to_celsius(temp_f)),
            target_c: Some(fahrenheit_to_celsius(target_f)),
            alert_high_c: None,
            alert_low_c: None,
            pre_alert_c: None,
            is_pit: false,
            range_c: None,
            stalled: false,
        }
    }

    fn snapshot(probes: Vec<ProbeSnapshot>) -> Snapshot {
        Snapshot {
            probes,
            battery: Some(80),
            last_reading_at: Some(AppInstant::ZERO),
            serve_late: false,
            alert_on_stall: true,
        }
    }

    #[test]
    fn target_reached_repeat_schedule() {
        let mut engine = AlertEngine::new(90, 2, 30);
        let mut seen = Vec::new();
        for second in 0..=900 {
            let now = AppInstant::from_secs_f64(f64::from(second));
            let snap = snapshot(vec![probe(1, 205.0, 203.0)]);
            for delivery in engine.evaluate(&snap, now).deliveries {
                if delivery.key
                    == AlertKey::new(Some(ProbeId::new(1).unwrap()), AlertKind::TargetReached)
                {
                    seen.push(second);
                }
            }
        }
        assert_eq!(seen, vec![0, 90, 270, 570, 870]);
    }

    #[test]
    fn ack_stops_repeats() {
        let mut engine = AlertEngine::new(90, 2, 30);
        let key = AlertKey::new(Some(ProbeId::new(1).unwrap()), AlertKind::TargetReached);
        let mut snap = snapshot(vec![probe(1, 205.0, 203.0)]);
        assert_eq!(engine.evaluate(&snap, AppInstant::ZERO).deliveries.len(), 1);
        engine.ack(key, &snap);
        let mut deliveries = 0;
        for second in 11..=5000 {
            let now = AppInstant::from_secs_f64(f64::from(second));
            snap.last_reading_at = Some(now);
            deliveries += engine
                .evaluate(&snap, now)
                .deliveries
                .iter()
                .filter(|delivery| delivery.key == key)
                .count();
        }
        assert_eq!(deliveries, 0);
    }

    #[test]
    fn dropping_below_target_clears() {
        let mut engine = AlertEngine::new(90, 2, 30);
        let snap_hot = snapshot(vec![probe(1, 205.0, 203.0)]);
        assert_eq!(
            engine
                .evaluate(&snap_hot, AppInstant::ZERO)
                .deliveries
                .len(),
            1
        );
        let snap_cool = snapshot(vec![probe(1, 200.0, 203.0)]);
        let now = AppInstant::from_secs_f64(30.0);
        assert!(engine.evaluate(&snap_cool, now).deliveries.is_empty());
        assert!(engine.active().is_empty());
    }

    #[test]
    fn different_keys_are_independent() {
        let mut engine = AlertEngine::new(90, 2, 30);
        let first = engine
            .evaluate(&snapshot(vec![probe(1, 205.0, 203.0)]), AppInstant::ZERO)
            .deliveries;
        assert_eq!(first.len(), 1);
        let both = snapshot(vec![probe(1, 205.0, 203.0), probe(2, 210.0, 203.0)]);
        let later = engine
            .evaluate(&both, AppInstant::from_secs_f64(10.0))
            .deliveries;
        assert_eq!(later.len(), 1, "second probe's alert must not be held back");
    }

    #[test]
    fn unplugged_alert_after_delay() {
        let mut engine = AlertEngine::new(90, 2, 30);
        let reading = ProbeSnapshot {
            alert_high_c: None,
            alert_low_c: None,
            ..probe(1, 150.0, 203.0)
        };
        assert!(
            engine
                .evaluate(
                    &snapshot(vec![reading.clone()]),
                    AppInstant::from_secs_f64(-1.0)
                )
                .deliveries
                .is_empty()
        );
        let mut unplugged = reading;
        unplugged.temp_c = None;
        let mut fired = false;
        for second in 0..=60 {
            let now = AppInstant::from_secs_f64(f64::from(second));
            for delivery in engine
                .evaluate(&snapshot(vec![unplugged.clone()]), now)
                .deliveries
            {
                if delivery.key.kind == AlertKind::ProbeUnplugged {
                    assert_eq!(second, 30);
                    fired = true;
                }
            }
        }
        assert!(fired);
    }

    #[test]
    fn cleared_alerts_reported_and_pruned() {
        let mut engine = AlertEngine::new(90, 2, 30);
        let key = AlertKey::new(Some(ProbeId::new(1).unwrap()), AlertKind::TargetReached);
        let hot = snapshot(vec![probe(1, 205.0, 203.0)]);
        assert_eq!(engine.evaluate(&hot, AppInstant::ZERO).deliveries.len(), 1);
        let cool = snapshot(vec![probe(1, 200.0, 203.0)]);
        let cleared = engine
            .evaluate(&cool, AppInstant::from_secs_f64(10.0))
            .cleared;
        assert_eq!(cleared, vec![(key, AppInstant::from_secs_f64(10.0))]);
        // Well past an hour, the old clear is pruned from the recent list...
        let later = AppInstant::from_secs_f64(4000.0);
        let _ = engine.evaluate(&cool, later);
        assert!(engine.cleared_recent(later).is_empty());
        // ...but a fresh clear is still reported.
        assert_eq!(engine.evaluate(&hot, later).deliveries.len(), 1);
        let cleared = engine
            .evaluate(&cool, AppInstant::from_secs_f64(4010.0))
            .cleared;
        assert_eq!(cleared.len(), 1, "new clears must still be reported");
    }
}
