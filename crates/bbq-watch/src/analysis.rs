//! History buffer, regression rate, stall detection and finish-time estimates.

use std::collections::VecDeque;

use crate::clock::AppInstant;
use crate::config::{CookSettings, EtaMethod};
use crate::model::{c_delta_to_f, f_delta_to_c};

/// Downsample interval: keep the last reading in each 10 simulated-second bucket.
const BUCKET_S: f64 = 10.0;
/// Samples older than 24 simulated hours are dropped.
const MAX_AGE_S: f64 = 24.0 * 3600.0;
/// Minimum samples needed for a regression.
const MIN_SAMPLES: usize = 10;
/// Minimum span needed for a regression.
const MIN_SPAN_S: f64 = 5.0 * 60.0;

/// Per-probe history of `(instant, temperature)`, downsampled to one sample per
/// 10 simulated seconds and capped at 24 hours.
#[derive(Debug, Default)]
pub struct ProbeHistory {
    samples: VecDeque<(AppInstant, Option<f32>)>,
}

impl ProbeHistory {
    /// An empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all samples.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Record a reading, collapsing repeats within the same 10 s bucket.
    pub fn push(&mut self, at: AppInstant, temp_c: Option<f32>) {
        let bucket = (at.secs() / BUCKET_S).floor() as i64;
        let same_bucket = self
            .samples
            .back()
            .is_some_and(|(last, _)| (last.secs() / BUCKET_S).floor() as i64 == bucket);
        if same_bucket {
            if let Some(back) = self.samples.back_mut() {
                *back = (at, temp_c);
            }
        } else {
            self.samples.push_back((at, temp_c));
        }
        self.trim();
    }

    fn trim(&mut self) {
        if let Some((last, _)) = self.samples.back() {
            let cutoff = last.secs() - MAX_AGE_S;
            while let Some((first, _)) = self.samples.front() {
                if first.secs() < cutoff {
                    self.samples.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    /// Raw samples, oldest first.
    #[must_use]
    pub fn samples(&self) -> &VecDeque<(AppInstant, Option<f32>)> {
        &self.samples
    }

    /// Number of stored samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Most recent non-`None` temperature.
    #[must_use]
    pub fn current(&self) -> Option<f32> {
        self.samples.iter().rev().find_map(|(_, temp)| *temp)
    }

    /// First non-`None` reading.
    #[must_use]
    pub fn first_some(&self) -> Option<(AppInstant, f32)> {
        self.samples
            .iter()
            .find_map(|(at, temp)| temp.map(|value| (*at, value)))
    }

    /// Minimum and maximum stored temperatures.
    #[must_use]
    pub fn min_max(&self) -> Option<(f32, f32)> {
        let mut values = self.samples.iter().filter_map(|(_, temp)| *temp);
        let first = values.next()?;
        Some(values.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
    }

    /// Least-squares rate over the last `window_min` minutes, in °C/h.
    #[must_use]
    pub fn rate_c_per_h(&self, now: AppInstant, window_min: f64) -> Option<f32> {
        let from = AppInstant::from_secs_f64(now.secs() - window_min * 60.0);
        self.rate_between_c_per_h(from, now)
    }

    /// Least-squares rate between two instants, in °C/h.
    #[must_use]
    pub fn rate_between_c_per_h(&self, from: AppInstant, to: AppInstant) -> Option<f32> {
        let points: Vec<(f64, f64)> = self
            .samples
            .iter()
            .filter(|(at, _)| *at >= from && *at <= to)
            .filter_map(|(at, temp)| temp.map(|value| (at.secs(), f64::from(value))))
            .collect();
        if points.len() < MIN_SAMPLES {
            return None;
        }
        let x0 = points.first()?.0;
        let span = points.last()?.0 - x0;
        if span < MIN_SPAN_S {
            return None;
        }
        let count = points.len() as f64;
        let mean_x = points.iter().map(|(x, _)| x - x0).sum::<f64>() / count;
        let mean_y = points.iter().map(|(_, y)| *y).sum::<f64>() / count;
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for (x, y) in &points {
            let dx = (x - x0) - mean_x;
            numerator += dx * (y - mean_y);
            denominator += dx * dx;
        }
        if denominator == 0.0 {
            return None;
        }
        Some(((numerator / denominator) * 3600.0) as f32)
    }

    /// Last non-`None` value at or before `at`.
    #[must_use]
    pub fn value_near(&self, at: AppInstant) -> Option<f32> {
        let mut found = None;
        for (sample_at, temp) in &self.samples {
            if *sample_at > at {
                break;
            }
            if temp.is_some() {
                found = *temp;
            }
        }
        found
    }

    /// Maximum deviation from the mean over the last `window_min` minutes, °C.
    #[must_use]
    pub fn max_deviation_c(&self, now: AppInstant, window_min: f64) -> Option<f32> {
        let from = AppInstant::from_secs_f64(now.secs() - window_min * 60.0);
        let values: Vec<f32> = self
            .samples
            .iter()
            .filter(|(at, _)| *at >= from && *at <= now)
            .filter_map(|(_, temp)| *temp)
            .collect();
        if values.is_empty() {
            return None;
        }
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        values
            .iter()
            .map(|value| (value - mean).abs())
            .reduce(f32::max)
    }
}

/// Stall state transitions reported by [`StallTracker::update`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StallTransition {
    /// No change.
    None,
    /// A stall just started.
    Started,
    /// A stall just ended.
    Ended,
}

/// Continuous-below-threshold stall detector for one probe.
#[derive(Debug, Default)]
pub struct StallTracker {
    below_since: Option<AppInstant>,
    above_since: Option<AppInstant>,
    started_at: Option<AppInstant>,
}

impl StallTracker {
    /// Instant the current stall began, if any.
    #[must_use]
    pub fn started_at(&self) -> Option<AppInstant> {
        self.started_at
    }

    /// Whether the probe is currently stalled.
    #[must_use]
    pub fn is_stalled(&self) -> bool {
        self.started_at.is_some()
    }

    /// Forget all state (e.g. on reconnect).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed a new rate and temperature; returns any transition.
    pub fn update(
        &mut self,
        now: AppInstant,
        temp_c: Option<f32>,
        rate_c_h: Option<f32>,
        cook: &CookSettings,
    ) -> StallTransition {
        let threshold = f_delta_to_c(cook.stall_rate_f_per_h);
        let double = f_delta_to_c(2.0 * cook.stall_rate_f_per_h);

        match rate_c_h {
            Some(rate) if rate < threshold => {
                if self.below_since.is_none() {
                    self.below_since = Some(now);
                }
            }
            _ => self.below_since = None,
        }
        match rate_c_h {
            Some(rate) if rate >= double => {
                if self.above_since.is_none() {
                    self.above_since = Some(now);
                }
            }
            _ => self.above_since = None,
        }

        let in_range = temp_c.is_some_and(|t| t >= cook.stall_min_c && t <= cook.stall_max_c);

        let mut ended = false;
        if self.started_at.is_some() {
            let above_long = self
                .above_since
                .is_some_and(|since| now.since(since) >= 10.0 * 60.0);
            let too_hot = temp_c.is_some_and(|t| t > cook.stall_max_c);
            if above_long || too_hot {
                self.started_at = None;
                ended = true;
            }
        }

        let mut started = false;
        if self.started_at.is_none()
            && let Some(since) = self.below_since
            && in_range
            && now.since(since) >= cook.stall_minutes * 60.0
        {
            self.started_at = Some(since);
            started = true;
        }

        if started {
            StallTransition::Started
        } else if ended {
            StallTransition::Ended
        } else {
            StallTransition::None
        }
    }
}

/// Finish-time estimate for one meat probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EtaResult {
    /// The probe has reached its target.
    pub done: bool,
    /// Estimated seconds remaining, or `None` when it cannot be estimated.
    pub seconds: Option<f64>,
    /// Whether the estimate used the stall-aware model.
    pub stall_adjusted: bool,
    /// What a straight-line fit would have given.
    pub linear_seconds: Option<f64>,
}

impl EtaResult {
    /// A result with no estimate at all.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            done: false,
            seconds: None,
            stall_adjusted: false,
            linear_seconds: None,
        }
    }
}

/// Inputs for a finish-time estimate.
pub struct EtaInput<'a> {
    /// Probe target in °C.
    pub target_c: f32,
    /// Current temperature in °C.
    pub current_c: Option<f32>,
    /// Recent rate in °C/h.
    pub rate_c_h: Option<f32>,
    /// Whether the probe is stalled.
    pub stalled: bool,
    /// When the stall began.
    pub stall_started_at: Option<AppInstant>,
    /// When the probe was last wrapped.
    pub wrapped_at: Option<AppInstant>,
    /// The probe's history.
    pub history: &'a ProbeHistory,
}

/// Compute a meat probe's ETA.
#[must_use]
pub fn compute_eta(input: &EtaInput<'_>, now: AppInstant, cook: &CookSettings) -> EtaResult {
    let target_c = input.target_c;
    let Some(current) = input.current_c else {
        return EtaResult::unknown();
    };

    if current >= target_c {
        return EtaResult {
            done: true,
            seconds: Some(0.0),
            stall_adjusted: false,
            linear_seconds: Some(0.0),
        };
    }

    let linear = linear_seconds(target_c, current, input.rate_c_h);
    let stall_aware = matches!(cook.eta_method, EtaMethod::StallAware) && input.stalled;
    if stall_aware && let Some(started) = input.stall_started_at {
        let wrapped = cook.wrapped_resets_stall && input.wrapped_at.is_some();
        let expected_min = if wrapped { 90.0 } else { 180.0 };
        let elapsed_min = now.since(started) / 60.0;
        let remaining_min = (expected_min - elapsed_min).max(15.0);
        let post_rate_c = f_delta_to_c(post_rate_f(input.history, started));
        let rise_seconds = f64::from((target_c - current) / post_rate_c) * 3600.0;
        return EtaResult {
            done: false,
            seconds: Some(remaining_min * 60.0 + rise_seconds),
            stall_adjusted: true,
            linear_seconds: linear,
        };
    }

    EtaResult {
        done: false,
        seconds: linear,
        stall_adjusted: false,
        linear_seconds: linear,
    }
}

/// Straight-line ETA in seconds, or `None` when the rate is too small.
fn linear_seconds(target_c: f32, current_c: f32, rate_c_h: Option<f32>) -> Option<f64> {
    let rate = rate_c_h?;
    if rate < f_delta_to_c(0.5) {
        return None;
    }
    Some(f64::from((target_c - current_c) / rate) * 3600.0)
}

/// Regression rate over the 60 minutes before the stall, clamped to `[8, 25]` °F/h.
fn post_rate_f(history: &ProbeHistory, stalled_at: AppInstant) -> f32 {
    let from = AppInstant::from_secs_f64(stalled_at.secs() - 3600.0);
    history
        .rate_between_c_per_h(from, stalled_at)
        .map_or(12.0, c_delta_to_f)
        .clamp(8.0, 25.0)
}

/// Format a duration as `5h 10m` (or `12m`, or `—` for `None`).
#[must_use]
pub fn format_duration(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds else {
        return String::from("—");
    };
    if seconds < 0.0 {
        return String::from("—");
    }
    let total_minutes = (seconds / 60.0).round() as i64;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::{celsius_to_fahrenheit, fahrenheit_to_celsius};

    fn sample(at_min: f64, temp_f: f32) -> (AppInstant, Option<f32>) {
        (
            AppInstant::from_secs_f64(at_min * 60.0),
            Some(fahrenheit_to_celsius(temp_f)),
        )
    }

    #[test]
    fn regression_rate_is_ten_f_per_hour() {
        let mut history = ProbeHistory::new();
        for minute in 0..=30 {
            let (at, temp) = sample(
                f64::from(minute),
                100.0 + 10.0 * f64::from(minute) as f32 / 60.0,
            );
            history.push(at, temp);
        }
        let now = AppInstant::from_secs_f64(30.0 * 60.0);
        let rate_f = c_delta_to_f(history.rate_c_per_h(now, 30.0).unwrap());
        assert!((rate_f - 10.0).abs() < 0.1, "rate was {rate_f}");
    }

    fn run_stall(flat_minutes: u32) -> bool {
        let cook = CookSettings::default();
        let mut history = ProbeHistory::new();
        let mut tracker = StallTracker::default();
        // Rise at 3.1 °F/h to 150 °F, then hold.
        let mut stalled = false;
        for minute in 0..=(30 + flat_minutes) {
            let temp_f = if minute <= 30 {
                150.0 - 3.1 * (30.0 - f64::from(minute)) as f32 / 60.0
            } else {
                150.0
            };
            let at = AppInstant::from_secs_f64(f64::from(minute) * 60.0);
            history.push(at, Some(fahrenheit_to_celsius(temp_f)));
            let rate = history.rate_c_per_h(at, cook.rate_window_min);
            let transition = tracker.update(at, Some(fahrenheit_to_celsius(temp_f)), rate, &cook);
            if transition == StallTransition::Started {
                stalled = true;
            }
        }
        stalled
    }

    #[test]
    fn stalls_after_twenty_five_minutes_flat() {
        assert!(run_stall(25), "expected a stall after 25 minutes flat");
    }

    #[test]
    fn does_not_stall_after_fifteen_minutes_flat() {
        assert!(!run_stall(15), "did not expect a stall after 15 minutes");
    }

    #[test]
    fn stall_aware_eta_matches_spec() {
        let cook = CookSettings::default();
        let history = ProbeHistory::new();
        let now = AppInstant::from_secs_f64(1000.0 * 60.0);
        let started = AppInstant::from_secs_f64(900.0 * 60.0);
        let target_c = fahrenheit_to_celsius(203.0);
        let current_c = fahrenheit_to_celsius(150.4);
        let result = compute_eta(
            &EtaInput {
                target_c,
                current_c: Some(current_c),
                rate_c_h: Some(f_delta_to_c(1.0)),
                stalled: true,
                stall_started_at: Some(started),
                wrapped_at: None,
                history: &history,
            },
            now,
            &cook,
        );
        assert!(result.stall_adjusted);
        let minutes = result.seconds.unwrap() / 60.0;
        assert!((minutes - 343.0).abs() <= 1.0, "minutes was {minutes}");
        assert!((celsius_to_fahrenheit(target_c) - 203.0).abs() < 0.01);
    }

    #[test]
    fn done_probe_reports_done() {
        let cook = CookSettings::default();
        let history = ProbeHistory::new();
        let now = AppInstant::from_secs_f64(60.0);
        let result = compute_eta(
            &EtaInput {
                target_c: fahrenheit_to_celsius(203.0),
                current_c: Some(fahrenheit_to_celsius(205.0)),
                rate_c_h: Some(1.0),
                stalled: false,
                stall_started_at: None,
                wrapped_at: None,
                history: &history,
            },
            now,
            &cook,
        );
        assert!(result.done);
    }

    #[test]
    fn history_downsampled_per_ten_seconds() {
        let mut history = ProbeHistory::new();
        for tenths in 0..25 {
            history.push(AppInstant::from_secs_f64(f64::from(tenths)), Some(1.0));
        }
        // 0..24 s -> buckets 0,1,2 -> three samples.
        assert_eq!(history.len(), 3);
    }
}
