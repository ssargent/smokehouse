//! Terminal dashboard and alerting for iBBQ Bluetooth thermometers.
//!
//! `bbq-watch` connects to an Inkbird IBT-4XS over BLE (or to a simulated
//! thermometer in demo mode), shows live probe temperatures with a history
//! chart and table and estimated finish times, and raises sound, banner, push
//! and spoken alerts.

pub mod alerts;
pub mod analysis;
pub mod app;
pub mod caffeinate;
pub mod cli;
pub mod clock;
pub mod config;
pub mod device;
pub mod logger;
pub mod model;
pub mod notify;
pub mod rng;
pub mod ui;
