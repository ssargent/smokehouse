//! Persistent settings: TOML load/save, defaults and cook presets.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{PROBE_COUNT, ProbeColor, Unit, f_delta_to_c, fahrenheit_to_celsius};

/// Errors raised while loading or saving settings.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// No platform config directory was available.
    #[error("no configuration directory is available on this platform")]
    NoDir,
    /// Filesystem failure.
    #[error("config i/o: {0}")]
    Io(#[from] std::io::Error),
    /// Serialisation failure.
    #[error("config encode: {0}")]
    Encode(#[from] toml::ser::Error),
    /// Parse failure.
    #[error("config parse: {0}")]
    Decode(#[from] toml::de::Error),
}

/// Which portion of the cook history the chart shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartWindow {
    /// The whole cook so far.
    #[default]
    #[serde(rename = "whole")]
    Whole,
    /// The last six hours.
    #[serde(rename = "6h")]
    SixHours,
    /// The last two hours.
    #[serde(rename = "2h")]
    TwoHours,
    /// The last thirty minutes.
    #[serde(rename = "30m")]
    ThirtyMinutes,
}

impl ChartWindow {
    /// All options, in display order.
    pub const ALL: [Self; 4] = [
        Self::Whole,
        Self::SixHours,
        Self::TwoHours,
        Self::ThirtyMinutes,
    ];

    /// Option label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Whole => "whole",
            Self::SixHours => "6h",
            Self::TwoHours => "2h",
            Self::ThirtyMinutes => "30m",
        }
    }

    /// Window length in simulated minutes, or `None` for the whole cook.
    #[must_use]
    pub fn minutes(self) -> Option<f64> {
        match self {
            Self::Whole => None,
            Self::SixHours => Some(360.0),
            Self::TwoHours => Some(120.0),
            Self::ThirtyMinutes => Some(30.0),
        }
    }
}

/// How estimated finish times are computed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EtaMethod {
    /// Account for the stall plateau.
    #[default]
    StallAware,
    /// Straight-line extrapolation of the current rate.
    Linear,
}

/// Which units the thermometer's own screen shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceScreenUnits {
    /// Follow the app's display unit.
    #[default]
    #[serde(rename = "match")]
    Match,
    /// Fahrenheit.
    #[serde(rename = "F")]
    Fahrenheit,
    /// Celsius.
    #[serde(rename = "C")]
    Celsius,
}

impl DeviceScreenUnits {
    /// All options, in display order.
    pub const ALL: [Self; 3] = [Self::Match, Self::Fahrenheit, Self::Celsius];

    /// Option label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Match => "Match app",
            Self::Fahrenheit => "°F",
            Self::Celsius => "°C",
        }
    }

    /// Resolve to a concrete unit given the app's display unit.
    #[must_use]
    pub fn resolve(self, display: Unit) -> Unit {
        match self {
            Self::Match => display,
            Self::Fahrenheit => Unit::Fahrenheit,
            Self::Celsius => Unit::Celsius,
        }
    }
}

/// Cook preset; choosing one sets the probe target (and pit range).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Brisket, probe-tender.
    Brisket,
    /// Beef ribs.
    BeefRibs,
    /// Pork shoulder, pulled.
    PorkPulled,
    /// Pork shoulder, sliced.
    PorkSliced,
    /// Pork ribs.
    PorkRibs,
    /// Pork loin.
    PorkLoin,
    /// Chicken breast or whole bird.
    Chicken,
    /// Chicken thighs.
    ChickenThigh,
    /// Turkey breast.
    TurkeyBreast,
    /// Beef, medium-rare.
    BeefMr,
    /// Salmon.
    Salmon,
    /// Pit / ambient probe.
    Pit,
    /// Leave the target untouched.
    #[default]
    Custom,
}

impl Preset {
    /// All presets, in display order.
    pub const ALL: [Self; 13] = [
        Self::Brisket,
        Self::BeefRibs,
        Self::PorkPulled,
        Self::PorkSliced,
        Self::PorkRibs,
        Self::PorkLoin,
        Self::Chicken,
        Self::ChickenThigh,
        Self::TurkeyBreast,
        Self::BeefMr,
        Self::Salmon,
        Self::Pit,
        Self::Custom,
    ];

    /// Menu label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Brisket => "Brisket · probe-tender",
            Self::BeefRibs => "Beef ribs",
            Self::PorkPulled => "Pork shoulder · pulled",
            Self::PorkSliced => "Pork shoulder · sliced",
            Self::PorkRibs => "Pork ribs",
            Self::PorkLoin => "Pork loin",
            Self::Chicken => "Chicken breast / whole",
            Self::ChickenThigh => "Chicken thighs",
            Self::TurkeyBreast => "Turkey breast",
            Self::BeefMr => "Beef · medium-rare",
            Self::Salmon => "Salmon",
            Self::Pit => "Pit / ambient",
            Self::Custom => "Custom",
        }
    }

    /// Target temperature in °C, when the preset defines one.
    #[must_use]
    pub fn target_c(self) -> Option<f32> {
        self.target_f().map(fahrenheit_to_celsius)
    }

    fn target_f(self) -> Option<f32> {
        match self {
            Self::Brisket | Self::BeefRibs => Some(203.0),
            Self::PorkPulled | Self::PorkRibs => Some(195.0),
            Self::PorkSliced => Some(185.0),
            Self::PorkLoin => Some(145.0),
            Self::Chicken => Some(165.0),
            Self::ChickenThigh => Some(175.0),
            Self::TurkeyBreast => Some(160.0),
            Self::BeefMr => Some(130.0),
            Self::Salmon => Some(140.0),
            Self::Pit | Self::Custom => None,
        }
    }

    /// True for the pit / ambient preset.
    #[must_use]
    pub fn is_pit(self) -> bool {
        matches!(self, Self::Pit)
    }

    /// Apply the preset to a probe config (target, and pit range and alerts).
    pub fn apply(self, probe: &mut ProbeConfig) {
        probe.preset = self;
        match self {
            Self::Pit => {
                probe.target_c = None;
                probe.range_c = Some([fahrenheit_to_celsius(225.0), fahrenheit_to_celsius(275.0)]);
                probe.alert_low_c = Some(fahrenheit_to_celsius(215.0));
                probe.alert_high_c = Some(fahrenheit_to_celsius(290.0));
            }
            Self::Custom => {}
            other => {
                probe.target_c = other.target_c();
            }
        }
    }
}

/// One probe's settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProbeConfig {
    /// Display label (max 18 characters in the editor).
    #[serde(default)]
    pub label: String,
    /// Cook preset.
    #[serde(default)]
    pub preset: Preset,
    /// Target temperature in °C (`None` for the pit preset).
    #[serde(default)]
    pub target_c: Option<f32>,
    /// Pit range `[min, max]` in °C (pit only).
    #[serde(default)]
    pub range_c: Option<[f32; 2]>,
    /// Low alarm threshold in °C.
    #[serde(default)]
    pub alert_low_c: Option<f32>,
    /// High alarm threshold in °C.
    #[serde(default)]
    pub alert_high_c: Option<f32>,
    /// Pre-alert distance below target in °C.
    #[serde(default)]
    pub pre_alert_c: Option<f32>,
    /// Chart and card colour.
    #[serde(default)]
    pub color: ProbeColor,
    /// Whether the probe is drawn on the chart.
    #[serde(default = "default_true")]
    pub show_on_chart: bool,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            label: String::from("Probe"),
            preset: Preset::Custom,
            target_c: None,
            range_c: None,
            alert_low_c: None,
            alert_high_c: None,
            pre_alert_c: Some(f_delta_to_c(5.0)),
            color: ProbeColor::White,
            show_on_chart: true,
        }
    }
}

/// `[display]` section.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplaySettings {
    /// Display unit.
    #[serde(default)]
    pub units: Unit,
    /// Chart window.
    #[serde(default)]
    pub chart_window: ChartWindow,
    /// Table row interval in minutes (1, 5 or 15).
    #[serde(default = "default_table_interval")]
    pub table_interval_min: u32,
    /// Redraw interval in milliseconds.
    #[serde(default = "default_refresh_ms")]
    pub refresh_ms: u64,
    /// Colour theme name.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Draw meat target lines on the chart.
    #[serde(default = "default_true")]
    pub show_target_lines: bool,
    /// Draw pit range lines on the chart.
    #[serde(default = "default_true")]
    pub show_pit_range_lines: bool,
    /// 24-hour clock.
    #[serde(default = "default_true")]
    pub clock_24h: bool,
    /// Compact probe cards.
    #[serde(default)]
    pub compact_cards: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            units: Unit::Fahrenheit,
            chart_window: ChartWindow::Whole,
            table_interval_min: default_table_interval(),
            refresh_ms: default_refresh_ms(),
            theme: default_theme(),
            show_target_lines: true,
            show_pit_range_lines: true,
            clock_24h: true,
            compact_cards: false,
        }
    }
}

/// `[alarms]` section.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlarmSettings {
    /// Play a sound on delivery.
    #[serde(default = "default_true")]
    pub sound: bool,
    /// Sound file name in `/System/Library/Sounds`.
    #[serde(default = "default_sound_file")]
    pub sound_file: String,
    /// Show a macOS banner.
    #[serde(default = "default_true")]
    pub banner: bool,
    /// Push to ntfy.
    #[serde(default)]
    pub ntfy: bool,
    /// ntfy topic.
    #[serde(default)]
    pub ntfy_topic: String,
    /// Terminal bell.
    #[serde(default)]
    pub bell: bool,
    /// Seconds before the first repeat.
    #[serde(default = "default_repeat_s")]
    pub repeat_s: u32,
    /// Snooze length in minutes.
    #[serde(default = "default_snooze_min")]
    pub snooze_min: u32,
    /// Seconds a probe must be unplugged before alerting.
    #[serde(default = "default_unplugged_after_s")]
    pub unplugged_after_s: u32,
    /// Minutes without readings before a Bluetooth-lost alert.
    #[serde(default = "default_bt_lost_after_min")]
    pub bt_lost_after_min: u32,
    /// Alert on stall start and end.
    #[serde(default = "default_true")]
    pub alert_on_stall: bool,
}

impl Default for AlarmSettings {
    fn default() -> Self {
        Self {
            sound: true,
            sound_file: default_sound_file(),
            banner: true,
            ntfy: false,
            ntfy_topic: String::new(),
            bell: false,
            repeat_s: default_repeat_s(),
            snooze_min: default_snooze_min(),
            unplugged_after_s: default_unplugged_after_s(),
            bt_lost_after_min: default_bt_lost_after_min(),
            alert_on_stall: true,
        }
    }
}

/// `[speech]` section.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpeechSettings {
    /// Whether spoken alerts are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `say` voice name (empty = system default).
    #[serde(default)]
    pub voice: String,
    /// Speaking rate in words per minute.
    #[serde(default = "default_rate_wpm")]
    pub rate_wpm: u32,
    /// `say` audio device (empty = default output).
    #[serde(default)]
    pub audio_device: String,
    /// Speak only urgent alerts.
    #[serde(default)]
    pub urgent_only: bool,
    /// Quiet-hours window, e.g. `23:00-07:00` (empty = off).
    #[serde(default)]
    pub quiet_hours: String,
    /// Speak a status summary every N minutes (0 = off).
    #[serde(default)]
    pub status_every_min: u32,
}

impl Default for SpeechSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            voice: String::new(),
            rate_wpm: default_rate_wpm(),
            audio_device: String::new(),
            urgent_only: false,
            quiet_hours: String::new(),
            status_every_min: 0,
        }
    }
}

/// `[cook]` section.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookSettings {
    /// ETA method.
    #[serde(default)]
    pub eta_method: EtaMethod,
    /// Regression window in minutes.
    #[serde(default = "default_rate_window_min")]
    pub rate_window_min: f64,
    /// Stall rate threshold in °F/h.
    #[serde(default = "default_stall_rate")]
    pub stall_rate_f_per_h: f32,
    /// Minutes below the stall rate before a stall is declared.
    #[serde(default = "default_stall_minutes")]
    pub stall_minutes: f64,
    /// Stall lower bound in °C.
    #[serde(default = "default_stall_min_c")]
    pub stall_min_c: f32,
    /// Stall upper bound in °C.
    #[serde(default = "default_stall_max_c")]
    pub stall_max_c: f32,
    /// Rest time in minutes.
    #[serde(default = "default_rest_min")]
    pub rest_min: f64,
    /// Target serve time, `HH:MM` (empty = off).
    #[serde(default)]
    pub want_to_eat_at: String,
    /// Wrapping resets the expected remaining stall.
    #[serde(default = "default_true")]
    pub wrapped_resets_stall: bool,
    /// Freeze ETAs while the lid is open.
    #[serde(default)]
    pub pause_eta_on_lid_open: bool,
}

impl Default for CookSettings {
    fn default() -> Self {
        Self {
            eta_method: EtaMethod::StallAware,
            rate_window_min: default_rate_window_min(),
            stall_rate_f_per_h: default_stall_rate(),
            stall_minutes: default_stall_minutes(),
            stall_min_c: default_stall_min_c(),
            stall_max_c: default_stall_max_c(),
            rest_min: default_rest_min(),
            want_to_eat_at: String::new(),
            wrapped_resets_stall: true,
            pause_eta_on_lid_open: false,
        }
    }
}

/// `[device]` section.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceSettings {
    /// Saved CoreBluetooth peripheral UUID.
    #[serde(default)]
    pub peripheral_id: String,
    /// User rename, shown instead of the advertised name.
    #[serde(default)]
    pub display_name: String,
    /// Reconnect automatically after a drop.
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    /// Battery poll interval in minutes.
    #[serde(default = "default_battery_check_min")]
    pub battery_check_min: u32,
    /// Units shown on the thermometer's own screen.
    #[serde(default)]
    pub device_screen_units: DeviceScreenUnits,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            peripheral_id: String::new(),
            display_name: String::new(),
            auto_reconnect: true,
            battery_check_min: default_battery_check_min(),
            device_screen_units: DeviceScreenUnits::Match,
        }
    }
}

/// `[data]` section.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataSettings {
    /// Write readings to CSV.
    #[serde(default = "default_true")]
    pub csv_enabled: bool,
    /// CSV directory (empty = `~/bbq-watch`).
    #[serde(default)]
    pub csv_dir: String,
    /// Sampling interval in seconds (1, 10 or 60).
    #[serde(default = "default_sample_interval")]
    pub sample_interval_s: u32,
    /// Keep the Mac awake while running.
    #[serde(default = "default_true")]
    pub keep_awake: bool,
}

impl Default for DataSettings {
    fn default() -> Self {
        Self {
            csv_enabled: true,
            csv_dir: String::new(),
            sample_interval_s: default_sample_interval(),
            keep_awake: true,
        }
    }
}

/// The full application settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// Display section.
    #[serde(default)]
    pub display: DisplaySettings,
    /// The four probe definitions.
    #[serde(default = "default_probes")]
    pub probes: Vec<ProbeConfig>,
    /// Alarms section.
    #[serde(default)]
    pub alarms: AlarmSettings,
    /// Speech section.
    #[serde(default)]
    pub speech: SpeechSettings,
    /// Cook section.
    #[serde(default)]
    pub cook: CookSettings,
    /// Device section.
    #[serde(default)]
    pub device: DeviceSettings,
    /// Data section.
    #[serde(default)]
    pub data: DataSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: DisplaySettings::default(),
            probes: default_probes(),
            alarms: AlarmSettings::default(),
            speech: SpeechSettings::default(),
            cook: CookSettings::default(),
            device: DeviceSettings::default(),
            data: DataSettings::default(),
        }
    }
}

impl Settings {
    /// The probe config at `index`, clamped to the produced probe set.
    #[must_use]
    pub fn probe(&self, index: usize) -> ProbeConfig {
        self.probes
            .get(index)
            .cloned()
            .unwrap_or_else(|| default_probe(index))
    }

    /// CSV directory, resolving the empty default to `~/bbq-watch`.
    #[must_use]
    pub fn csv_dir(&self) -> PathBuf {
        if self.data.csv_dir.trim().is_empty() {
            default_csv_dir()
        } else {
            PathBuf::from(self.data.csv_dir.trim())
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_probes() -> Vec<ProbeConfig> {
    (0..PROBE_COUNT).map(default_probe).collect()
}

fn default_probe(index: usize) -> ProbeConfig {
    match index {
        0 => ProbeConfig {
            label: String::from("Brisket Point"),
            preset: Preset::Brisket,
            target_c: Preset::Brisket.target_c(),
            color: ProbeColor::Red,
            ..ProbeConfig::default()
        },
        1 => ProbeConfig {
            label: String::from("Brisket Flat"),
            preset: Preset::Brisket,
            target_c: Preset::Brisket.target_c(),
            color: ProbeColor::Orange,
            ..ProbeConfig::default()
        },
        2 => ProbeConfig {
            label: String::from("Pork Butt"),
            preset: Preset::PorkPulled,
            target_c: Preset::PorkPulled.target_c(),
            color: ProbeColor::Green,
            ..ProbeConfig::default()
        },
        _ => {
            let mut probe = ProbeConfig {
                label: String::from("Pit"),
                color: ProbeColor::Blue,
                ..ProbeConfig::default()
            };
            Preset::Pit.apply(&mut probe);
            probe
        }
    }
}

fn default_theme() -> String {
    String::from("tokyo-night")
}

const fn default_table_interval() -> u32 {
    5
}

const fn default_refresh_ms() -> u64 {
    1000
}

fn default_sound_file() -> String {
    String::from("Glass.aiff")
}

const fn default_repeat_s() -> u32 {
    90
}

const fn default_snooze_min() -> u32 {
    10
}

const fn default_unplugged_after_s() -> u32 {
    30
}

const fn default_bt_lost_after_min() -> u32 {
    2
}

const fn default_rate_wpm() -> u32 {
    180
}

const fn default_rate_window_min() -> f64 {
    30.0
}

const fn default_stall_rate() -> f32 {
    3.0
}

const fn default_stall_minutes() -> f64 {
    20.0
}

fn default_stall_min_c() -> f32 {
    fahrenheit_to_celsius(145.0)
}

fn default_stall_max_c() -> f32 {
    fahrenheit_to_celsius(175.0)
}

const fn default_rest_min() -> f64 {
    60.0
}

const fn default_battery_check_min() -> u32 {
    5
}

const fn default_sample_interval() -> u32 {
    10
}

/// Platform config directory for `bbq-watch`.
pub fn project_dirs() -> Result<directories::ProjectDirs, ConfigError> {
    directories::ProjectDirs::from("", "", "bbq-watch").ok_or(ConfigError::NoDir)
}

/// Directory holding the config file and the log file.
pub fn data_dir() -> Result<PathBuf, ConfigError> {
    Ok(project_dirs()?.data_dir().to_path_buf())
}

/// Default CSV directory (`~/bbq-watch`).
#[must_use]
pub fn default_csv_dir() -> PathBuf {
    directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join("bbq-watch"))
        .unwrap_or_else(|| PathBuf::from("bbq-watch"))
}

/// Resolve the config path, honouring an explicit override.
pub fn config_path(override_path: Option<&Path>) -> Result<PathBuf, ConfigError> {
    match override_path {
        Some(path) => Ok(path.to_path_buf()),
        None => Ok(project_dirs()?.config_dir().join("config.toml")),
    }
}

/// Load settings, creating the file with defaults when it is missing.
pub fn load(path: &Path) -> Result<Settings, ConfigError> {
    if !path.exists() {
        return create_default(path);
    }
    let text = std::fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return create_default(path);
    }
    Ok(toml::from_str(&text)?)
}

/// Write settings atomically (temp file, then rename).
pub fn save(settings: &Settings, path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(settings)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn create_default(path: &Path) -> Result<Settings, ConfigError> {
    let mut settings = Settings::default();
    settings.alarms.ntfy_topic = format!("bbq-watch-{}", random_hex4());
    save(&settings, path)?;
    Ok(settings)
}

/// Four random-ish hex digits for a first-run ntfy topic.
fn random_hex4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    let mut x = nanos ^ (u64::from(std::process::id()) << 32) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    format!("{:04x}", (x & 0xFFFF) as u16)
}

/// Parse a `HH:MM` string into `(hour, minute)`.
#[must_use]
pub fn parse_hhmm(value: &str) -> Option<(u32, u32)> {
    let (hours, minutes) = value.trim().split_once(':')?;
    let hour: u32 = hours.trim().parse().ok()?;
    let minute: u32 = minutes.trim().parse().ok()?;
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// Format `(hour, minute)` as `HH:MM`.
#[must_use]
pub fn format_hhmm(hour: u32, minute: u32) -> String {
    format!("{hour:02}:{minute:02}")
}

/// Whether `hour:minute` falls inside a `HH:MM-HH:MM` window (which may wrap
/// past midnight). An empty or malformed window is never quiet.
#[must_use]
pub fn in_quiet_hours(window: &str, hour: u32, minute: u32) -> bool {
    let Some((start, end)) = window.trim().split_once('-') else {
        return false;
    };
    let (Some((start_h, start_m)), Some((end_h, end_m))) = (parse_hhmm(start), parse_hhmm(end))
    else {
        return false;
    };
    let now = hour * 60 + minute;
    let start = start_h * 60 + start_m;
    let end = end_h * 60 + end_m;
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_four_probes() {
        let settings = Settings::default();
        assert_eq!(settings.probes.len(), PROBE_COUNT);
        assert_eq!(settings.probes[0].label, "Brisket Point");
        assert!(settings.probes[3].preset.is_pit());
        assert!(settings.probes[3].target_c.is_none());
        assert!(settings.probes[3].range_c.is_some());
    }

    #[test]
    fn round_trips_through_toml() {
        let settings = Settings::default();
        let text = toml::to_string_pretty(&settings).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.probes.len(), PROBE_COUNT);
        assert_eq!(back.alarms.repeat_s, 90);
        assert_eq!(back.display.units, Unit::Fahrenheit);
    }

    #[test]
    fn loads_partial_config() {
        let partial = "[data]\ncsv_enabled = false\n";
        let settings: Settings = toml::from_str(partial).unwrap();
        assert!(!settings.data.csv_enabled);
        assert_eq!(settings.alarms.repeat_s, 90);
        assert_eq!(settings.probes.len(), PROBE_COUNT);
    }

    #[test]
    fn preset_sets_target() {
        let mut probe = ProbeConfig::default();
        Preset::PorkPulled.apply(&mut probe);
        assert_eq!(probe.preset, Preset::PorkPulled);
        let target_f = crate::model::celsius_to_fahrenheit(probe.target_c.unwrap());
        assert!((target_f - 195.0).abs() < 0.01);
    }

    #[test]
    fn first_run_creates_file_with_topic() {
        let dir = std::env::temp_dir().join(format!("bbq-watch-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let _ = std::fs::remove_dir_all(&dir);
        let settings = load(&path).unwrap();
        assert!(path.exists());
        assert!(settings.alarms.ntfy_topic.starts_with("bbq-watch-"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
