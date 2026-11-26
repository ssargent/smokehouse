//! Core model types: probe identity, temperature units and conversions.

use serde::{Deserialize, Serialize};

/// Number of probes an iBBQ thermometer exposes.
pub const PROBE_COUNT: usize = 4;

/// Identifier for one probe, numbered `P1`..`P4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProbeId(u8);

impl ProbeId {
    /// Build a `ProbeId` from a 1-based probe number.
    #[must_use]
    pub fn new(number: u8) -> Option<Self> {
        (1..=PROBE_COUNT as u8)
            .contains(&number)
            .then_some(Self(number))
    }

    /// Zero-based index into per-probe arrays.
    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.0 - 1)
    }

    /// 1-based probe number.
    #[must_use]
    pub fn number(self) -> u8 {
        self.0
    }

    /// Build from a zero-based index (`0..PROBE_COUNT`).
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self((index as u8) + 1)
    }

    /// All four probes in order.
    #[must_use]
    pub fn all() -> [Self; PROBE_COUNT] {
        [Self(1), Self(2), Self(3), Self(4)]
    }
}

impl std::fmt::Display for ProbeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P{}", self.0)
    }
}

/// Temperature unit used for display and input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    /// Fahrenheit.
    #[default]
    #[serde(rename = "F")]
    Fahrenheit,
    /// Celsius.
    #[serde(rename = "C")]
    Celsius,
}

impl Unit {
    /// Unit letter.
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Fahrenheit => "F",
            Self::Celsius => "C",
        }
    }

    /// Convert an internal Celsius value to this unit.
    #[must_use]
    pub fn from_c(self, celsius: f32) -> f32 {
        match self {
            Self::Fahrenheit => celsius_to_fahrenheit(celsius),
            Self::Celsius => celsius,
        }
    }

    /// Convert a value in this unit to internal Celsius.
    #[must_use]
    pub fn to_c(self, value: f32) -> f32 {
        match self {
            Self::Fahrenheit => fahrenheit_to_celsius(value),
            Self::Celsius => value,
        }
    }

    /// Convert an internal Celsius *difference* to this unit.
    #[must_use]
    pub fn delta_from_c(self, delta_c: f32) -> f32 {
        match self {
            Self::Fahrenheit => c_delta_to_f(delta_c),
            Self::Celsius => delta_c,
        }
    }

    /// Convert a difference in this unit to internal Celsius.
    #[must_use]
    pub fn delta_to_c(self, delta: f32) -> f32 {
        match self {
            Self::Fahrenheit => f_delta_to_c(delta),
            Self::Celsius => delta,
        }
    }

    /// Format an internal Celsius temperature for display.
    #[must_use]
    pub fn format_c(self, celsius: f32) -> String {
        format!("{:.1}°{}", self.from_c(celsius), self.symbol())
    }

    /// Format an internal Celsius temperature as a rounded whole number.
    #[must_use]
    pub fn format_c_whole(self, celsius: f32) -> String {
        format!("{:.0}", self.from_c(celsius))
    }
}

/// Celsius to Fahrenheit.
#[must_use]
pub fn celsius_to_fahrenheit(celsius: f32) -> f32 {
    celsius * 9.0 / 5.0 + 32.0
}

/// Fahrenheit to Celsius.
#[must_use]
pub fn fahrenheit_to_celsius(fahrenheit: f32) -> f32 {
    (fahrenheit - 32.0) * 5.0 / 9.0
}

/// A Fahrenheit *difference* to a Celsius difference.
#[must_use]
pub fn f_delta_to_c(delta_f: f32) -> f32 {
    delta_f * 5.0 / 9.0
}

/// A Celsius *difference* to a Fahrenheit difference.
#[must_use]
pub fn c_delta_to_f(delta_c: f32) -> f32 {
    delta_c * 9.0 / 5.0
}

/// Colours a probe card may use; names match the config file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeColor {
    /// Red.
    Red,
    /// Orange.
    Orange,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Purple.
    Purple,
    /// Cyan.
    Cyan,
    /// Yellow.
    Yellow,
    /// White (also the default).
    #[default]
    White,
}

impl ProbeColor {
    /// All colours in cycle order.
    pub const ALL: [Self; 8] = [
        Self::Red,
        Self::Orange,
        Self::Green,
        Self::Blue,
        Self::Purple,
        Self::Cyan,
        Self::Yellow,
        Self::White,
    ];

    /// Config name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Cyan => "cyan",
            Self::Yellow => "yellow",
            Self::White => "white",
        }
    }

    /// Next colour in the cycle.
    #[must_use]
    pub fn next(self) -> Self {
        let all = Self::ALL;
        let pos = all.iter().position(|c| *c == self).unwrap_or(0);
        all[(pos + 1) % all.len()]
    }
}
