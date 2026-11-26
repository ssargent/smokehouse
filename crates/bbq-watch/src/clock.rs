//! Application clock: simulates time so the demo can be accelerated.

use std::time::Instant;

use chrono::{DateTime, Duration as ChronoDuration, Local};

/// Simulated elapsed seconds since the application started.
///
/// With `speed == 1` this is real elapsed time. With `speed == 60` one real
/// second advances the clock by one simulated minute.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct AppInstant(f64);

impl AppInstant {
    /// Zero simulated seconds.
    pub const ZERO: Self = Self(0.0);

    /// Build from a simulated-seconds value.
    #[must_use]
    pub fn from_secs_f64(secs: f64) -> Self {
        Self(secs)
    }

    /// Simulated seconds since the app started.
    #[must_use]
    pub fn secs(self) -> f64 {
        self.0
    }

    /// Simulated minutes since the app started.
    #[must_use]
    pub fn minutes(self) -> f64 {
        self.0 / 60.0
    }

    /// Simulated seconds elapsed from `earlier` to `self`.
    #[must_use]
    pub fn since(self, earlier: Self) -> f64 {
        self.0 - earlier.0
    }

    /// A point `minutes` simulated minutes after `self`.
    #[must_use]
    pub fn plus_minutes(self, minutes: f64) -> Self {
        Self(self.0 + minutes * 60.0)
    }
}

impl std::ops::Add<f64> for AppInstant {
    type Output = Self;

    fn add(self, secs: f64) -> Self {
        Self(self.0 + secs)
    }
}

/// Source of simulated time, abstracted so tests can use a fixed clock.
pub trait Clock: Send + Sync {
    /// Current simulated instant.
    fn now(&self) -> AppInstant;

    /// Wall-clock time corresponding to a simulated instant.
    fn wall(&self, t: AppInstant) -> DateTime<Local>;
}

/// Real, optionally accelerated clock.
#[derive(Clone, Debug)]
pub struct AppClock {
    start_real: Instant,
    start_wall: DateTime<Local>,
    speed: f64,
}

impl AppClock {
    /// Start a clock that runs `speed` simulated seconds per real second.
    #[must_use]
    pub fn new(speed: f64) -> Self {
        let speed = if speed.is_finite() && speed > 0.0 {
            speed
        } else {
            1.0
        };
        Self {
            start_real: Instant::now(),
            start_wall: Local::now(),
            speed,
        }
    }

    /// Simulated seconds per real second.
    #[must_use]
    pub fn speed(&self) -> f64 {
        self.speed
    }
}

impl Clock for AppClock {
    fn now(&self) -> AppInstant {
        AppInstant(self.start_real.elapsed().as_secs_f64() * self.speed)
    }

    fn wall(&self, t: AppInstant) -> DateTime<Local> {
        let millis = (t.secs() * 1000.0) as i64;
        self.start_wall + ChronoDuration::milliseconds(millis)
    }
}

/// A clock frozen at a fixed instant, advanced explicitly by tests.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock {
    now: AppInstant,
    start_wall: DateTime<Local>,
}

impl FixedClock {
    /// A clock starting at zero simulated seconds on `start_wall`.
    #[must_use]
    pub fn new(start_wall: DateTime<Local>) -> Self {
        Self {
            now: AppInstant::ZERO,
            start_wall,
        }
    }

    /// Advance the clock by `seconds`.
    pub fn advance(&mut self, seconds: f64) {
        self.now = AppInstant::from_secs_f64(self.now.secs() + seconds);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> AppInstant {
        self.now
    }

    fn wall(&self, t: AppInstant) -> DateTime<Local> {
        let millis = (t.secs() * 1000.0) as i64;
        self.start_wall + ChronoDuration::milliseconds(millis)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    #[test]
    fn fixed_clock_maps_instants_to_wall_time() {
        let start = Local.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let mut clock = FixedClock::new(start);
        assert_eq!(clock.now(), AppInstant::ZERO);
        assert_eq!(clock.wall(AppInstant::ZERO), start);
        clock.advance(90.0);
        assert_eq!(clock.now(), AppInstant::from_secs_f64(90.0));
        assert_eq!(clock.wall(clock.now()).format("%H:%M").to_string(), "12:01");
    }
}
