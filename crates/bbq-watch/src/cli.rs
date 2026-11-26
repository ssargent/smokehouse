//! Command-line arguments.

use std::path::PathBuf;

use clap::Parser;

/// Terminal dashboard and alerting for iBBQ Bluetooth thermometers.
#[derive(Debug, Clone, Parser)]
#[command(name = "bbq-watch", version, about, long_about = None)]
pub struct Cli {
    /// Use the simulated thermometer instead of BLE.
    #[arg(long)]
    pub demo: bool,

    /// Demo time multiplier (one real second equals `speed` simulated seconds).
    #[arg(long, default_value_t = 1.0)]
    pub speed: f64,

    /// Override the configuration file path.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Force spoken alerts off for this run.
    #[arg(long)]
    pub no_speech: bool,
}
