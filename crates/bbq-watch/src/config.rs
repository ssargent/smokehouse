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

/// Reserved id of the built-in Custom pseudo-preset.
pub const CUSTOM_PRESET_ID: &str = "custom";
/// Display name of the built-in Custom pseudo-preset.
pub const CUSTOM_PRESET_NAME: &str = "Custom";

/// The built-in Custom pseudo-preset: always available, never stored in
/// `[[presets]]`, and never editable or deletable.
#[must_use]
pub fn custom_preset() -> &'static Preset {
    static CUSTOM: std::sync::LazyLock<Preset> = std::sync::LazyLock::new(|| Preset {
        id: String::from(CUSTOM_PRESET_ID),
        name: String::from(CUSTOM_PRESET_NAME),
        ..Preset::default()
    });
    &CUSTOM
}

/// Cook preset; choosing one sets the probe target (and pit range).
///
/// Presets live in the config file (`[[presets]]`), so they can be edited and
/// extended from the Settings tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// Stable key referenced by `[[probes]] preset`.
    #[serde(default)]
    pub id: String,
    /// Menu label.
    #[serde(default)]
    pub name: String,
    /// Target temperature in °F (`None` = leave the target untouched).
    #[serde(default)]
    pub target_f: Option<f32>,
    /// Pit / ambient preset: apply the range and alert bounds below instead.
    #[serde(default)]
    pub pit: bool,
    /// Pit range in °F, `[low, high]`.
    #[serde(default)]
    pub range_f: Option<[f32; 2]>,
    /// Pit low-alert threshold in °F.
    #[serde(default)]
    pub alert_low_f: Option<f32>,
    /// Pit high-alert threshold in °F.
    #[serde(default)]
    pub alert_high_f: Option<f32>,
}

impl Preset {
    /// Apply the preset to a probe config (target, and pit range and alerts).
    pub fn apply(&self, probe: &mut ProbeConfig) {
        probe.preset.clone_from(&self.id);
        if self.pit {
            probe.target_c = None;
            if let Some([low, high]) = self.range_f {
                probe.range_c = Some([fahrenheit_to_celsius(low), fahrenheit_to_celsius(high)]);
            }
            if let Some(low) = self.alert_low_f {
                probe.alert_low_c = Some(fahrenheit_to_celsius(low));
            }
            if let Some(high) = self.alert_high_f {
                probe.alert_high_c = Some(fahrenheit_to_celsius(high));
            }
        } else if let Some(target) = self.target_f {
            probe.target_c = Some(fahrenheit_to_celsius(target));
        }
    }

    /// A config-file-friendly id for a preset name.
    #[must_use]
    pub fn slugify(name: &str) -> String {
        let mut slug = String::new();
        for character in name.chars() {
            if character.is_ascii_alphanumeric() {
                slug.push(character.to_ascii_lowercase());
            } else if !slug.is_empty() && !slug.ends_with('_') {
                slug.push('_');
            }
        }
        while slug.ends_with('_') {
            slug.pop();
        }
        if slug.is_empty() {
            slug.push_str("preset");
        }
        slug
    }
}

/// The presets a fresh config starts with.
fn default_presets() -> Vec<Preset> {
    let meat = |id: &str, name: &str, target_f: f32| Preset {
        id: String::from(id),
        name: String::from(name),
        target_f: Some(target_f),
        ..Preset::default()
    };
    vec![
        meat("brisket", "Brisket · probe-tender", 203.0),
        meat("beef_ribs", "Beef ribs", 203.0),
        meat("pork_pulled", "Pork shoulder · pulled", 195.0),
        meat("pork_sliced", "Pork shoulder · sliced", 185.0),
        meat("pork_ribs", "Pork ribs", 195.0),
        meat("pork_loin", "Pork loin", 145.0),
        meat("chicken", "Chicken breast / whole", 165.0),
        meat("chicken_thigh", "Chicken thighs", 175.0),
        meat("turkey_breast", "Turkey breast", 160.0),
        meat("beef_mr", "Beef · medium-rare", 130.0),
        meat("salmon", "Salmon", 140.0),
        Preset {
            id: String::from("pit"),
            name: String::from("Pit / ambient"),
            pit: true,
            range_f: Some([225.0, 275.0]),
            alert_low_f: Some(215.0),
            alert_high_f: Some(290.0),
            ..Preset::default()
        },
    ]
}

/// One probe's settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProbeConfig {
    /// Display label (max 18 characters in the editor).
    #[serde(default)]
    pub label: String,
    /// Cook preset id, resolved against [`Settings::presets`].
    #[serde(default = "default_preset_id")]
    pub preset: String,
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
            preset: default_preset_id(),
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
    /// Cook presets, editable from the Settings tab.
    #[serde(default = "default_presets")]
    pub presets: Vec<Preset>,
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
            presets: default_presets(),
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

    /// The preset with `id`, when the config defines it (including the
    /// built-in Custom pseudo-preset).
    #[must_use]
    pub fn preset(&self, id: &str) -> Option<&Preset> {
        if id == CUSTOM_PRESET_ID {
            return Some(custom_preset());
        }
        self.presets.iter().find(|preset| preset.id == id)
    }

    /// Display name for the preset with `id`, falling back to the id itself.
    #[must_use]
    pub fn preset_name(&self, id: &str) -> String {
        self.preset(id)
            .map_or_else(|| id.to_string(), |preset| preset.name.clone())
    }

    /// Whether probe `index` uses the pit / ambient preset.
    #[must_use]
    pub fn is_pit(&self, index: usize) -> bool {
        let id = self.probes.get(index).map_or_else(
            || default_probe_preset(index),
            |probe| probe.preset.as_str(),
        );
        self.preset(id).is_some_and(|preset| preset.pit)
    }

    /// Apply a preset to a probe: copy its values and record that the probe
    /// follows it. Custom leaves the values alone and only clears the link.
    pub fn apply_preset(&mut self, index: usize, preset_id: &str) {
        let Some(preset) = self.preset(preset_id).cloned() else {
            return;
        };
        let Some(probe) = self.probes.get_mut(index) else {
            return;
        };
        if preset_id == CUSTOM_PRESET_ID {
            probe.preset = String::from(CUSTOM_PRESET_ID);
        } else {
            preset.apply(probe);
        }
    }

    /// Update every probe that follows the preset at `index` to its values.
    pub fn sync_preset_followers(&mut self, index: usize) {
        let Some(preset) = self.presets.get(index).cloned() else {
            return;
        };
        for probe in &mut self.probes {
            if probe.preset == preset.id {
                preset.apply(probe);
            }
        }
    }

    /// Mark a probe as hand-edited: it no longer follows a preset.
    pub fn mark_custom(&mut self, index: usize) {
        if let Some(probe) = self.probes.get_mut(index) {
            probe.preset = String::from(CUSTOM_PRESET_ID);
        }
    }

    /// Remove the preset at `index`. Probes that followed it become Custom,
    /// keeping their values; they are never reassigned to another preset.
    pub fn remove_preset(&mut self, index: usize) {
        if index >= self.presets.len() {
            return;
        }
        let removed = self.presets.remove(index);
        for probe in &mut self.probes {
            if probe.preset == removed.id {
                probe.preset = String::from(CUSTOM_PRESET_ID);
            }
        }
    }

    /// `base`, or `base_2`, `base_3`… when another preset already uses it.
    /// `ignore` is the index of the preset being renamed.
    #[must_use]
    pub fn unique_preset_id(&self, base: &str, ignore: Option<usize>) -> String {
        let taken = |candidate: &str| {
            candidate == CUSTOM_PRESET_ID
                || self
                    .presets
                    .iter()
                    .enumerate()
                    .any(|(index, preset)| preset.id == candidate && Some(index) != ignore)
        };
        if !taken(base) {
            return base.to_string();
        }
        for suffix in 2..1000 {
            let candidate = format!("{base}_{suffix}");
            if !taken(&candidate) {
                return candidate;
            }
        }
        base.to_string()
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

/// Preset id a probe falls back to when the config omits its probes.
fn default_probe_preset(index: usize) -> &'static str {
    match index {
        0 | 1 => "brisket",
        2 => "pork_pulled",
        _ => "pit",
    }
}

fn default_probe(index: usize) -> ProbeConfig {
    let (label, color) = match index {
        0 => ("Brisket Point", ProbeColor::Red),
        1 => ("Brisket Flat", ProbeColor::Orange),
        2 => ("Pork Butt", ProbeColor::Green),
        _ => ("Pit", ProbeColor::Blue),
    };
    let mut probe = ProbeConfig {
        label: String::from(label),
        color,
        ..ProbeConfig::default()
    };
    let preset = default_probe_preset(index);
    if let Some(found) = default_presets()
        .into_iter()
        .find(|entry| entry.id == preset)
    {
        found.apply(&mut probe);
    }
    probe
}

/// Preset id a probe starts with when the config omits one.
fn default_preset_id() -> String {
    String::from("custom")
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
    let mut settings: Settings = toml::from_str(&text)?;
    settings
        .presets
        .retain(|preset| preset.id != CUSTOM_PRESET_ID);
    if settings.presets.is_empty() {
        settings.presets = default_presets();
    }
    Ok(settings)
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
        assert!(settings.is_pit(3));
        assert!(!settings.is_pit(0));
        assert!(settings.probes[3].target_c.is_none());
        assert!(settings.probes[3].range_c.is_some());
        assert_eq!(settings.preset_name("brisket"), "Brisket · probe-tender");
    }

    #[test]
    fn round_trips_through_toml() {
        let settings = Settings::default();
        let text = toml::to_string_pretty(&settings).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.probes.len(), PROBE_COUNT);
        assert_eq!(back.presets.len(), settings.presets.len());
        assert_eq!(back.presets[0].id, "brisket");
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
        assert_eq!(settings.presets.len(), 12);
    }

    #[test]
    fn loads_legacy_preset_names() {
        let legacy =
            "[[probes]]\nlabel = \"Brisket Point\"\npreset = \"brisket\"\ntarget_c = 95.0\n";
        let settings: Settings = toml::from_str(legacy).unwrap();
        assert_eq!(settings.probes[0].preset, "brisket");
        assert_eq!(
            settings.preset_name(&settings.probes[0].preset),
            "Brisket · probe-tender"
        );
        assert!(!settings.is_pit(0));
    }

    #[test]
    fn preset_sets_target() {
        let settings = Settings::default();
        let mut probe = ProbeConfig::default();
        settings.preset("pork_pulled").unwrap().apply(&mut probe);
        assert_eq!(probe.preset, "pork_pulled");
        let target_f = crate::model::celsius_to_fahrenheit(probe.target_c.unwrap());
        assert!((target_f - 195.0).abs() < 0.01);
    }

    #[test]
    fn pit_preset_sets_range_and_alerts() {
        let settings = Settings::default();
        let mut probe = ProbeConfig::default();
        settings.preset("pit").unwrap().apply(&mut probe);
        assert_eq!(probe.preset, "pit");
        assert!(probe.target_c.is_none());
        let range = probe.range_c.unwrap();
        let fahrenheit = crate::model::celsius_to_fahrenheit;
        assert!((fahrenheit(range[0]) - 225.0).abs() < 0.01);
        assert!((fahrenheit(range[1]) - 275.0).abs() < 0.01);
        assert!((fahrenheit(probe.alert_low_c.unwrap()) - 215.0).abs() < 0.01);
        assert!((fahrenheit(probe.alert_high_c.unwrap()) - 290.0).abs() < 0.01);
    }

    #[test]
    fn preset_edits_persist_through_save_and_load() {
        let dir = std::env::temp_dir().join(format!("bbq-watch-presets-{}", std::process::id()));
        let path = dir.join("config.toml");
        let _ = std::fs::remove_dir_all(&dir);
        let mut settings = load(&path).unwrap();
        settings.presets[0].name = String::from("Brisket · 205");
        settings.presets[0].target_f = Some(205.0);
        settings.presets.push(Preset {
            id: String::from("picanha"),
            name: String::from("Picanha"),
            target_f: Some(135.0),
            ..Preset::default()
        });
        settings.probes[0].preset = String::from("picanha");
        save(&settings, &path).unwrap();

        let reloaded = load(&path).unwrap();
        assert_eq!(reloaded.preset_name("brisket"), "Brisket · 205");
        assert_eq!(reloaded.preset("brisket").unwrap().target_f, Some(205.0));
        assert_eq!(reloaded.preset("picanha").unwrap().target_f, Some(135.0));
        assert_eq!(reloaded.probes[0].preset, "picanha");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preset_ids_slugify_names() {
        assert_eq!(Preset::slugify("Picanha"), "picanha");
        assert_eq!(
            Preset::slugify("Brisket · probe-tender"),
            "brisket_probe_tender"
        );
        assert_eq!(Preset::slugify("  Hot & Fast  "), "hot_fast");
        assert_eq!(Preset::slugify("°°"), "preset");
    }

    #[test]
    fn preset_ids_avoid_collisions() {
        let settings = Settings::default();
        assert_eq!(settings.unique_preset_id("picanha", None), "picanha");
        assert_eq!(settings.unique_preset_id("brisket", None), "brisket_2");
        // Renaming a preset may keep its own id.
        assert_eq!(settings.unique_preset_id("brisket", Some(0)), "brisket");
    }

    #[test]
    fn applying_preset_links_probe_and_copies_values() {
        let mut settings = Settings::default();
        settings.apply_preset(0, "pork_pulled");
        assert_eq!(settings.probes[0].preset, "pork_pulled");
        let target_f = crate::model::celsius_to_fahrenheit(settings.probes[0].target_c.unwrap());
        assert!((target_f - 195.0).abs() < 0.01);
    }

    #[test]
    fn editing_preset_updates_followers() {
        let mut settings = Settings::default();
        settings.apply_preset(0, "brisket");
        settings.apply_preset(1, "brisket");
        let index = settings
            .presets
            .iter()
            .position(|preset| preset.id == "brisket")
            .unwrap();
        settings.presets[index].target_f = Some(208.0);
        settings.sync_preset_followers(index);
        for row in [0, 1] {
            let target_f =
                crate::model::celsius_to_fahrenheit(settings.probes[row].target_c.unwrap());
            assert!((target_f - 208.0).abs() < 0.01);
        }
    }

    #[test]
    fn hand_editing_probe_makes_it_custom() {
        let mut settings = Settings::default();
        settings.apply_preset(0, "brisket");
        settings.mark_custom(0);
        assert_eq!(settings.probes[0].preset, CUSTOM_PRESET_ID);
        let index = settings
            .presets
            .iter()
            .position(|preset| preset.id == "brisket")
            .unwrap();
        settings.presets[index].target_f = Some(208.0);
        settings.sync_preset_followers(index);
        let target_f = crate::model::celsius_to_fahrenheit(settings.probes[0].target_c.unwrap());
        assert!((target_f - 203.0).abs() < 0.01);
    }

    #[test]
    fn deleting_preset_turns_followers_custom_keeping_values() {
        let mut settings = Settings::default();
        settings.apply_preset(0, "brisket");
        let before = settings.probes[0].target_c;
        let index = settings
            .presets
            .iter()
            .position(|preset| preset.id == "brisket")
            .unwrap();
        settings.remove_preset(index);
        assert_eq!(settings.probes[0].preset, CUSTOM_PRESET_ID);
        assert_eq!(settings.probes[0].target_c, before);
        assert!(settings.preset("brisket").is_none());
        assert!(settings.preset(CUSTOM_PRESET_ID).is_some());
    }

    #[test]
    fn custom_is_builtin_and_never_stored_or_reused() {
        let settings = Settings::default();
        assert!(
            settings
                .presets
                .iter()
                .all(|preset| preset.id != CUSTOM_PRESET_ID)
        );
        assert!(settings.preset(CUSTOM_PRESET_ID).is_some());
        assert_ne!(settings.unique_preset_id("custom", None), CUSTOM_PRESET_ID);
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
