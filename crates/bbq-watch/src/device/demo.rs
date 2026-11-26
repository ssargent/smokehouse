//! Simulated thermometer: a realistic cook on the app clock.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::clock::{AppClock, Clock};
use crate::device::{DeviceCommand, DeviceConfig, DeviceEvent};
use crate::model::{PROBE_COUNT, fahrenheit_to_celsius};
use crate::rng::Rng;

/// Round a Fahrenheit value to Celsius tenths, as the hardware reports it.
fn tenths_c(fahrenheit: f32) -> f32 {
    let celsius = fahrenheit_to_celsius(fahrenheit);
    (celsius * 10.0).round() / 10.0
}

/// Pit probe temperature in °F at `t` simulated minutes.
fn pit_f(t: f64) -> f64 {
    // Underdamped second-order warm-up from 72 to ~252 °F over ~25 min.
    let zeta_omega = 0.125_f64;
    let omega_d = zeta_omega * (1.0 - 0.7 * 0.7_f64).sqrt() / 0.7;
    let envelope = (-zeta_omega * t).exp();
    let response = envelope * ((omega_d * t).cos() + (zeta_omega / omega_d) * (omega_d * t).sin());
    let mut value = 252.0 - 180.0 * response;
    value += 3.5 * (t / 11.0).sin();
    if t >= 171.0 {
        // Lid open: drop 34 °F, recover exponentially (τ = 3.2 min).
        value -= 34.0 * (-((t - 171.0) / 3.2)).exp();
    }
    value
}

/// Meat trajectory with an asymptote, an optional late linear push and a cap.
fn meat_f(t: f64, start: f64, asymptote: f64, tau: f64, push_at: f64, cap: f64) -> f64 {
    let base = start + (asymptote - start) * (1.0 - (-t / tau).exp());
    if t <= push_at {
        base
    } else {
        let at_push = start + (asymptote - start) * (1.0 - (-push_at / tau).exp());
        (at_push + 12.0 * (t - push_at) / 60.0).min(cap)
    }
}

/// Pork butt trajectory: asymptote plus a slow late ramp, capped at 200 °F.
fn pork_butt_f(t: f64) -> f64 {
    let base = 40.0 + 116.0 * (1.0 - (-t / 74.0).exp()) + 0.177 * (t - 285.0).max(0.0);
    base.min(200.0)
}

/// The simulated cook.
struct DemoSim {
    rng: Rng,
}

impl DemoSim {
    fn new() -> Self {
        Self {
            rng: Rng::new(0x5EED_1234_ABCD_0001),
        }
    }

    fn readings(&mut self, t_min: f64) -> [Option<f32>; PROBE_COUNT] {
        let pit = pit_f(t_min) as f32 + self.rng.uniform(1.6);
        let point = meat_f(t_min, 38.0, 152.2, 84.0, 400.0, 205.0) as f32 + self.rng.uniform(0.25);
        let flat = meat_f(t_min, 37.0, 151.9, 80.0, 420.0, 205.0) as f32 + self.rng.uniform(0.25);
        let butt = pork_butt_f(t_min) as f32 + self.rng.uniform(0.25);
        [
            Some(tenths_c(point)),
            Some(tenths_c(flat)),
            Some(tenths_c(butt)),
            Some(tenths_c(pit)),
        ]
    }

    fn battery_pct(&self, elapsed_s: f64) -> u8 {
        let dropped = (elapsed_s / 1800.0).floor() as i32;
        u8::try_from((82 - dropped).max(0)).unwrap_or(0)
    }

    fn rssi(&self, t_min: f64) -> i16 {
        (-61.0 + 3.0 * (t_min / 23.0).sin()).round() as i16
    }
}

/// Run the simulated device until told to shut down.
pub async fn run(
    cfg: DeviceConfig,
    clock: AppClock,
    tx: mpsc::Sender<DeviceEvent>,
    mut cmd: mpsc::Receiver<DeviceCommand>,
) {
    let mut sim = DemoSim::new();
    let mut unplugged = [false; PROBE_COUNT];

    if tx
        .send(DeviceEvent::Connected {
            name: String::from("iBBQ-demo"),
            id: String::from("DEMO"),
        })
        .await
        .is_err()
    {
        return;
    }

    let step = f64::from(cfg.sample_interval_s.max(1));
    let real_sleep = (step / cfg.speed.max(1.0)).clamp(0.02, 5.0);
    let mut ticker = tokio::time::interval(Duration::from_secs_f64(real_sleep));
    let mut next_battery_s = 0.0_f64;
    let mut next_rssi_s = 0.0_f64;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = clock.now();
                let mut readings = sim.readings(now.minutes());
                for (slot, plugged) in readings.iter_mut().zip(unplugged) {
                    if plugged {
                        *slot = None;
                    }
                }
                if tx.send(DeviceEvent::Readings { at: now, temps_c: readings }).await.is_err() {
                    return;
                }
                let secs = now.secs();
                if secs >= next_rssi_s {
                    let _ = tx.send(DeviceEvent::Rssi(sim.rssi(now.minutes()))).await;
                    next_rssi_s = secs + 10.0;
                }
                if secs >= next_battery_s {
                    let _ = tx.send(DeviceEvent::Battery(sim.battery_pct(secs))).await;
                    next_battery_s = secs + 1800.0;
                }
            }
            incoming = cmd.recv() => {
                match incoming {
                    None | Some(DeviceCommand::Shutdown) => return,
                    Some(DeviceCommand::SetDeviceUnits(_)) => {}
                    Some(DeviceCommand::DemoTogglePlug(id)) => {
                        let index = id.index();
                        if let Some(slot) = unplugged.get_mut(index) {
                            *slot = !*slot;
                        }
                    }
                    Some(DeviceCommand::Rescan | DeviceCommand::Forget) => {
                        let _ = tx.send(DeviceEvent::Scanning).await;
                        let _ = tx.send(DeviceEvent::Connected {
                            name: String::from("iBBQ-demo"),
                            id: String::from("DEMO"),
                        }).await;
                    }
                }
            }
        }
    }
}
