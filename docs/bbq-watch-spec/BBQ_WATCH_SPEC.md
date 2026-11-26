# bbq-watch — Build Specification

`bbq-watch` is a macOS terminal app, written in Rust, that connects to an **Inkbird IBT-4XS** Bluetooth BBQ thermometer. It shows live probe temperatures, a history chart and table, and estimated finish times, and it sends alerts, including spoken alerts through macOS `say`.

It is a **new binary crate in the existing `smokehouse` Cargo workspace** (https://github.com/ssargent/smokehouse), at `crates/bbq-watch`.

This document is the complete spec. Build everything in it, in the order given in §14. Where this spec makes a decision, follow it. Don't redesign. Where it is silent, choose the simplest idiomatic option that satisfies the acceptance criteria in §15.

Reference mockups, if they're provided alongside this file: `mockups/bbq-watch-dashboard.png`, `mockups/bbq-watch-table.png` and `mockups/bbq-watch-settings.png`. The text layouts in this document are authoritative; the images only show the intended look.

---

## 0. Repository rules (read first)

### 0.1 What you may change
You may change **only** these:
1. Anything under `crates/bbq-watch/`, which is new and yours.
2. The root `Cargo.toml`, and only in two ways:
   - add `"crates/bbq-watch"` to `[workspace] members`, keeping the list alphabetical (it goes last, after `crates/bbq-telemetry`);
   - add `bbq-watch`'s dependency versions under `[workspace.dependencies]` (see §2). The repo's own comment there says shared versions go in that table.
3. `Cargo.lock`, as Cargo updates it.

Do **not** modify any other crate (`bbq-protocol`, `bbq-core`, `bbq-config` and the rest). That includes not adding `bbq-watch` as a dependency of them, and not making `bbq-watch` depend on them. They are empty placeholders today, so everything `bbq-watch` needs, including the iBBQ protocol code, lives inside `bbq-watch`. Don't change `[workspace.package]`, `[workspace.lints]`, `README.md`, `.gitignore`, `apps/`, `deploy/`, `docs/` or `migrations/`. Don't add `rustfmt.toml`, `clippy.toml` or `rust-toolchain.toml` anywhere.

### 0.2 Crate manifest
`crates/bbq-watch/Cargo.toml` follows the same template as the other crates, with one deliberate difference: `rust-version`. The workspace declares 1.85, but the current versions of ratatui and uuid need 1.89. Override it for this crate only, so the other crates are unaffected:

```toml
[package]
name = "bbq-watch"
version = "0.1.0"
description = "Terminal dashboard and alerting for iBBQ Bluetooth thermometers (macOS)"
edition.workspace = true
rust-version = "1.89"          # overrides workspace 1.85: ratatui 0.30 and uuid 1.29 need ≥1.88/1.89
license.workspace = true
repository.workspace = true
authors.workspace = true
publish.workspace = true

[dependencies]
anyhow.workspace = true
btleplug.workspace = true
# ... every dependency as `name.workspace = true` (features set in the root table, see §2)

[lints]
workspace = true
```

### 0.3 Workspace lints you must satisfy
The workspace sets `unsafe_code = "forbid"` and `unused_must_use = "deny"`. It also warns on clippy's `unwrap_used`, `expect_used`, `panic`, `todo`, `dbg_macro`, `print_stdout` and `print_stderr`. Treat every warning as an error: the gate is `cargo clippy -p bbq-watch --all-targets -- -D warnings`. In practice:
- **No `unsafe`.** Nothing in this app needs it.
- **No `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, `println!` or `eprintln!` in non-test code.** Propagate errors with `?`. Use `anyhow::Context` at the app edges and `thiserror` enums in modules that return typed errors (`device::ibbq`, `config`). Where an invariant truly can't fail, restructure it (e.g. `if let`, `let … else`, `.get()` with a fallback) instead of reaching for `expect`.
- **Test modules** may unwrap. Put this attribute on each one:
  ```rust
  #[cfg(test)]
  #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
  mod tests { … }
  ```
- **Never print to stdout or stderr** once the TUI starts. Diagnostics go through `tracing` to a log file (§10). `clap`'s own `--help`/`--version` output is fine.
- **Panic handling**: `ratatui::init()` already installs a panic hook that restores the terminal before the default hook prints. Use it, and don't write your own hook that calls `eprintln!`.
- Every `Result` and `#[must_use]` value must be handled. Intentionally ignoring a result is written `let _ = …;` with a short comment saying why (e.g. a best-effort `caffeinate` kill on exit).

### 0.4 Style: idiomatic, nothing exotic
- Use stable Rust only, with standard `rustfmt` formatting (`cargo fmt --all -- --check` must pass) and default clippy plus the workspace lints.
- Because the crate's `rust-version` is 1.89 and the edition is 2024, clippy's `collapsible_if` lint **requires let-chains**. Write `if let Some(x) = a && cond { … }`, not nested `if let { if … }`.
- Keep the structure plain: modules and structs, and enums for state and messages. Use traits only where tests need a seam: the speech "speaker" (§8.1) and the clock (§6.1). Don't build generic frameworks, macro DSLs, actor libraries or global mutable state, and don't use `lazy_static`/`once_cell` singletons.
- Use one tokio runtime (`#[tokio::main]`) and talk between tasks with `tokio::sync::mpsc`. The only shared state is the small `Arc<Mutex<…>>` active-alerts snapshot described in §8.1.
- Split the crate as a **thin `main.rs` plus a `lib.rs`** that declares the modules. This keeps the logic unit-testable and lets you add integration tests under `crates/bbq-watch/tests/` if you need them.
- Keep public items minimal. `pub(crate)` is the default for anything not used by `main.rs`.
- Write doc comments (`///`) on public types and on any non-obvious function. Add a short module-level `//!` comment at the top of each file.
- Add a `crates/bbq-watch/README.md` covering what the app is, how to run it (including `--demo --speed 60`), the macOS Bluetooth permission note, and where the config, CSV and log files live.

---

## 1. Goals and non-goals

**Goals (v1)**
- Read 4 probes from an iBBQ-protocol thermometer over BLE on macOS.
- Provide a ratatui terminal UI with 5 tabs: Dashboard, Table, Alarms, Log and Settings.
- Show a stall-aware estimated finish time (ETA) for each meat probe.
- Raise alerts by sound, macOS notification, ntfy push and spoken voice (`say`), using a prioritised speech queue with a cooldown for each alert.
- Persist settings to TOML and log readings to CSV.
- Provide a **`--demo` mode** with a simulated thermometer, so that everything except the BLE layer can be developed and tested without hardware.

**Non-goals (v1)**: MQTT/Home Assistant publishing, integration with the other smokehouse crates (server, Pi agent and so on), Windows/Linux BLE support (the crate must still *compile* on Linux, see §13), a history-download characteristic (`0xFFF3`), and multiple simultaneous thermometers.

---

## 2. Dependencies

Use the newest stable versions. The set below was resolved on 2026-11-25 and checked: it compiles together, with `cargo clippy -- -D warnings` passing under this workspace's lints, using the APIs this spec calls for. Use these versions (or newer patch/minor releases if `cargo update` picks them, as long as the gate in §15 still passes). Older versions aren't acceptable, because some APIs differ (see the notes).

Add them to the **root** `[workspace.dependencies]` table, with their features, then reference them from `crates/bbq-watch/Cargo.toml` as `name.workspace = true`:

```toml
[workspace.dependencies]
# Add shared dependency versions here as crates are created.
anyhow = "1.0"
btleplug = "0.13"
chrono = "0.4"
clap = { version = "4.6", features = ["derive"] }
crossterm = { version = "0.29", features = ["event-stream"] }
csv = "1.4"
directories = "6.0"
futures = "0.3"
ratatui = "0.30"
reqwest = { version = "0.13", default-features = false, features = ["rustls"] }
serde = { version = "1.0", features = ["derive"] }
thiserror = "2.0"
tokio = { version = "1.54", features = ["full"] }
toml = "1.2"
tracing = "0.1"
tracing-appender = "0.2"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = "1.29"
```
Keep the existing comment line and list the entries alphabetically.

**Edit the root `Cargo.toml` by hand.** Don't use `cargo add` or `cargo remove` for these entries. In testing, `cargo remove` deleted the then-empty `[workspace.dependencies]` header and left its comment stranded under `[workspace.lints.clippy]`, which breaks the manifest. After editing, check that the file still reads `[workspace.dependencies]`, then the comment, then the entries.

| Crate | Purpose | Notes |
|---|---|---|
| `btleplug` 0.13 | BLE via CoreBluetooth | The API used in §5 (`Manager`, `Central::events`, `start_scan(ScanFilter::default())`, `Peripheral::{connect, discover_services, characteristics, subscribe, write, notifications, properties}`, `bleuuid::uuid_from_u16`) is unchanged from 0.11. |
| `ratatui` 0.30 | TUI | Use `ratatui::init()` / `ratatui::restore()`. Import crossterm types **through `ratatui::crossterm`**. |
| `crossterm` 0.29 | Only to turn on the `event-stream` feature (`EventStream`) | It **must** be the same version ratatui uses. Check with `cargo tree -p bbq-watch -d`: there must be no duplicate `crossterm`. If ratatui moves to a newer crossterm, match it. |
| `tokio` | runtime, `process::Command`, `mpsc`, `time` | |
| `futures` | `StreamExt` | |
| `uuid` | GATT UUIDs | |
| `serde` + `toml` | config | |
| `csv` | reading log | |
| `chrono` | local wall-clock times | |
| `directories` | config/data paths | |
| `reqwest` 0.13 | ntfy push | In 0.13 the TLS feature is called **`rustls`**, not `rustls-tls`. Build the body as a `String`; the `json` feature isn't needed. |
| `anyhow` / `thiserror` | errors | |
| `clap` (derive) | CLI | |
| `tracing`, `tracing-subscriber`, `tracing-appender` | file logging | Never log to stdout. |

Don't add any other crates. If you're convinced one is essential, prefer writing a small helper yourself.

---

## 3. CLI

```
bbq-watch [--demo] [--speed <N>] [--config <path>] [--no-speech]
```
Run it from the workspace with `cargo run -p bbq-watch -- --demo --speed 60`.
- `--demo`: use the simulated thermometer (§5.4) instead of BLE.
- `--speed N`: demo time multiplier (default `1`, so `--speed 60` makes one real second equal one simulated minute). Every time-based calculation (rates, stall timers, cooldowns, ETAs, chart axis) must use the **app clock** (§6.1), so demo acceleration stays consistent.
- `--config`: overrides the config path.
- `--no-speech`: forces spoken alerts off for this run.

---

## 4. Crate layout

```
crates/bbq-watch/
  Cargo.toml
  README.md
  src/
    main.rs            // thin: parse CLI, init tracing, build runtime, call app::run
    lib.rs             // `pub mod` declarations only (plus crate-level //! docs)
    cli.rs             // clap Args struct
    clock.rs           // AppClock (real or accelerated) + Clock trait for tests
    model.rs           // ProbeId, Reading, Temperature helpers, units
    config.rs          // Settings structs, load/save TOML, presets
    device/
      mod.rs           // DeviceEvent, DeviceCommand, spawn_device(...)
      ibbq.rs          // protocol constants + pure parse/encode fns (unit tested)
      ble.rs           // btleplug implementation (scan/connect/reconnect)
      demo.rs          // simulated thermometer
    analysis.rs        // history buffer, regression rate, stall detection, ETA (unit tested)
    alerts.rs          // alert engine: conditions, hysteresis, cooldown, ack (unit tested)
    notify/
      mod.rs           // Notifier fan-out
      speech.rs        // speech queue worker (`say`) + Speaker trait
      sound.rs         // afplay
      banner.rs        // osascript display notification
      ntfy.rs          // HTTP push
    logger.rs          // CSV writer
    caffeinate.rs      // keep-awake child process
    app.rs             // App state + event loop + key handling
    ui/
      mod.rs           // top-level draw: header, tabs, footer, banner
      dashboard.rs
      table.rs
      alarms.rs
      log.rs
      settings.rs
      theme.rs         // colors
```

---

## 5. Device layer

### 5.1 iBBQ protocol (put this in `device/ibbq.rs`)

Everything lives under GATT service **`0xFFF0`** (use `btleplug::api::bleuuid::uuid_from_u16`).

| UUID | Name | Use |
|---|---|---|
| `0xFFF1` | SettingsResult | notify: command replies (battery) |
| `0xFFF2` | AccountAndVerify | write: login |
| `0xFFF4` | RealtimeData | notify: probe temperatures |
| `0xFFF5` | SettingsData | write: commands |

Byte sequences:
```
LOGIN            -> FFF2: 21 07 06 05 04 03 02 01 B8 22 00 00 00 00 00
ENABLE_REALTIME  -> FFF5: 0B 01 00 00 00 00
UNITS_CELSIUS    -> FFF5: 02 00 00 00 00 00
UNITS_FAHRENHEIT -> FFF5: 02 01 00 00 00 00
REQUEST_BATTERY  -> FFF5: 08 24 00 00 00 00
```

**Connection sequence**:
1. Connect and discover services.
2. Get the notifications stream *before* subscribing.
3. Write LOGIN. This must be the first write.
4. Subscribe to FFF4 and FFF1.
5. Write ENABLE_REALTIME.
6. Write the device-screen units, following the setting (§9, `device_screen_units`; "match app" means the app's display unit).
7. Write REQUEST_BATTERY, then repeat it every `battery_check_min`.

For writes, use `WriteType::WithResponse` if the characteristic has `CharPropFlags::WRITE`, otherwise `WithoutResponse`.

**Realtime payload (FFF4)**: one little-endian `u16` per probe, so the probe count is `len / 2`. The value is **tenths of a degree Celsius, always**, whatever the device's screen shows. Interpret it as `i16` before dividing by 10. `0xFFF6` means the probe is unplugged.

**Battery reply (FFF1)**: byte0 = `0x24`; bytes 1–2 = current voltage (u16 LE); bytes 3–4 = max voltage (u16 LE, and if it's 0, use 6550). Percent = `round(clamp(current / max * 100, 0, 100))`. Ignore other FFF1 messages.

Pure functions to implement and unit test:
```rust
pub fn parse_realtime(data: &[u8]) -> Vec<Option<f32>>; // °C, None = unplugged
pub fn parse_battery(data: &[u8]) -> Option<u8>;        // percent
```

### 5.2 Device interface (`device/mod.rs`)

```rust
pub enum DeviceEvent {
    Scanning,
    Connected { name: String, id: String },
    Disconnected { reason: String },
    Readings { at: AppInstant, temps_c: Vec<Option<f32>> }, // index 0 = P1
    Battery(u8),
    Rssi(i16),
    Error(String),
}
pub enum DeviceCommand { SetDeviceUnits(Unit), Rescan, Forget, Shutdown }

pub fn spawn_device(cfg: DeviceConfig, clock: AppClock, demo: bool)
    -> (mpsc::Receiver<DeviceEvent>, mpsc::Sender<DeviceCommand>);
```
The device task runs on its own and never blocks the UI.

### 5.3 BLE implementation (`device/ble.rs`)
- Scan with `ScanFilter::default()`. On macOS, filtering by service UUID is unreliable for this device. A device matches if its `local_name` is `"iBBQ"` or `"BBQ"`, **or** its peripheral id equals the saved `device.peripheral_id`. Prefer the saved id when it's present.
- macOS has **no MAC addresses**. `peripheral.id().to_string()` is a CoreBluetooth UUID. Save it to config after the first successful connect.
- Scan timeout is 30 s. Then emit `Error("not found")`, wait 10 s and scan again (forever, unless Shutdown).
- **Reconnect**: when the notification stream ends or a write fails, emit `Disconnected`. If `auto_reconnect` is on, retry with backoff of 5 s, 10 s, 20 s, 30 s, then 30 s forever.
- Poll RSSI from `properties().rssi` every 10 s while connected, if it's available.
- `Forget` clears the saved peripheral id and rescans.

### 5.4 Demo thermometer (`device/demo.rs`)
This must produce a realistic cook on the app clock. Emit one reading per simulated second (or every `sample_interval`). Start at app start, with 4 probes:
- **P4 pit**: rises from 72°F to about 252°F over about 25 simulated minutes with a small overshoot, then 252 ± (3.5·sin(t/11 min) + uniform ±1.6). At t = 171 min it drops by 34°F, recovering exponentially (τ = 3.2 min). That's a "lid open".
- **P1 brisket point**: `38 + (152.2-38)(1-e^(-t/84min))` °F plus ±0.25 noise. After t = 400 min, it continues upward at +12°F/h until it reaches 205°F.
- **P2 brisket flat**: the same with 37 / 151.9 / 80 min, breaking out at t = 420 min.
- **P3 pork butt**: `40 + 116(1-e^(-t/74min)) + 0.177·max(0, t-285min)` °F (capped at 200°F).
- The demo emits Celsius tenths, exactly like the hardware, so conversions are exercised. Battery starts at 82% and drops 1% every 30 simulated minutes. RSSI is −61 ± 3. In demo mode, `!` `@` `#` `$` (Shift+1–4) toggle probes P1–P4 "unplugged", so you can test disconnect alerts. Don't use Ctrl+digit: many terminals don't report it reliably.

---

## 6. Core model and analysis

### 6.1 Clock
```rust
#[derive(Clone)] pub struct AppClock { start_real: Instant, start_wall: DateTime<Local>, speed: f64 }
impl AppClock { fn now(&self) -> AppInstant; fn wall(&self, t: AppInstant) -> DateTime<Local>; }
```
`AppInstant` is a newtype around simulated elapsed seconds (`f64`). Use it everywhere instead of `Instant`/`SystemTime`. With speed 1 it's real time.

### 6.2 Units
Store and compute all temperatures **in °C (`f32`)** internally. Convert only for display and for user input. When the display unit is °F, the +/- keys adjust targets by 1°F (or 5°F with Shift) and convert back.

### 6.3 History
Each probe has a `VecDeque<(AppInstant, Option<f32>)>` holding at most 24 h of samples, downsampled to **one sample per 10 simulated seconds** (keep the last reading in each 10 s bucket). This feeds the chart, the table and the analysis.

### 6.4 Rate (°/h)
The rate is the least-squares linear-regression slope over the last `rate_window_min` minutes (default 30) of non-`None` samples. It needs at least 10 samples spanning at least 5 minutes, otherwise it is `None`. Convert to °C per hour, and display it in the chosen unit.

### 6.5 Stall detection (meat probes only, i.e. any probe whose preset isn't "Pit / ambient")
- A probe is **stalled** when its current temperature is within `[stall_min, stall_max]` (default 145–175°F) **and** its rate has stayed below `stall_rate` (default 3°F/h) continuously for `stall_minutes` (default 20 min).
- `stall_started_at` is the time the rate first went below the threshold in that continuous run.
- The stall **ends** when the rate stays at or above `2 × stall_rate` for 10 minutes, or the temperature goes above `stall_max`.
- Marking a probe "wrapped" (key `w` on the dashboard, applied to the selected probe) sets `wrapped_at = now`. If the setting `wrapped_resets_stall` is on, it also halves the expected remaining stall (see §6.6).

### 6.6 ETA
Each meat probe has a target `T`, current temperature `c` and rate `r`:
- If `c >= T`, the probe is **Done** and the card shows "DONE".
- **Linear ETA**: if `r` is `None` or `r < 0.5°F/h`, show "—"; otherwise `ETA = (T - c) / r`.
- **Stall-aware ETA** (the default method). When the probe is not stalled, use the linear ETA. When it is stalled:
  - `expected_stall = 180 min` (halved to 90 min if it's wrapped and `wrapped_resets_stall` is on);
  - `remaining_stall = max(expected_stall - (now - stall_started_at), 15 min)`;
  - `post_rate =` the regression rate over the 60 minutes before `stall_started_at`, clamped to `[8, 25] °F/h` (use 12°F/h if there isn't enough data);
  - `ETA = remaining_stall + (T - c) / post_rate`.
  - Flag it as stall-adjusted. The UI appends `*`, and the ETA panel footnote shows what the linear ETA would have been.
- **Auto-pause while lid open** (setting, default off): if the pit has dropped more than 25°F in the past 3 minutes, freeze the displayed ETAs at their last values until the pit recovers to within 10°F of its pre-drop value.
- **All done** = the latest meat-probe ETA. **Serve at** = all done + `rest_min`.
- If `want_to_eat_at` is set and serve time is later than it, raise a `ServeLate` alert (§7) and show the serve time in red.

Express ETAs as a duration (`5h 10m`) and a wall-clock time (`≈ 23:25`).

---

## 7. Alerts engine (`alerts.rs`)

Alerts are **level-triggered conditions** that are evaluated on every reading. Each has a stable key:

```rust
#[derive(Hash, Eq, PartialEq, Clone, Copy)]
pub enum AlertKind { TargetReached, PreAlert, AboveHigh, BelowLow, ProbeUnplugged,
                     BluetoothLost, StallStarted, StallEnded, BatteryLow, ServeLate }
pub type AlertKey = (Option<ProbeId>, AlertKind);
pub enum Priority { Urgent, Normal }
```

| Kind | Fires when | Priority | Clears when |
|---|---|---|---|
| TargetReached | temp ≥ target | Urgent | temp < target − 2°F (hysteresis) |
| PreAlert | temp ≥ target − pre_alert, and < target | Normal | temp < target − pre_alert − 2°F, or TargetReached fires |
| AboveHigh | temp > alert_high (pit uses range max if alert_high unset) | Urgent | temp < high − 2°F |
| BelowLow | temp < alert_low (pit uses range min); **only after** the probe has first been above the low threshold this cook (avoids alerts during warm-up) | Urgent | temp > low + 2°F |
| ProbeUnplugged | probe was reading, then `None` for ≥ `unplugged_after_s` | Urgent | readings return |
| BluetoothLost | no `Readings` event for ≥ `bt_lost_after_min` | Urgent | readings return |
| StallStarted | stall detected (§6.5), and setting `alert_on_stall` is on | Normal | one-shot |
| StallEnded | stall ends, and `alert_on_stall` is on | Normal | one-shot |
| BatteryLow | battery < 20% | Normal | battery ≥ 25% |
| ServeLate | serve time > want_to_eat_at | Normal | no longer late |

**Behaviour of each active alert:**
- When it first becomes active, it is **delivered** immediately to all enabled channels (§8).
- **Repeats** (not for one-shots): the alert is re-delivered while it stays active and isn't acknowledged. The gaps between deliveries escalate per key: the first repeat comes `repeat_s` after the initial delivery (default 90 s), the next `2 × repeat_s` after that (180 s), then `max(300 s, 3.33 × repeat_s)` after that, and that last gap repeats from then on. The cooldown applies to **each key separately**, never globally: a different alert is never held back by a recent one.
- **Acknowledge**: the `x` key on any tab acknowledges all active alerts, and `Enter` on the Alarms tab acknowledges the selected one. An acknowledged alert doesn't repeat. For a temperature alert, the ack is cancelled and the alert re-delivered if the temperature moves another 5°F further in the alarming direction. The ack also clears whenever the alert clears.
- **Snooze**: `z` snoozes all active alerts for `snooze_min`.
- Every fire, clear and ack is written to the event log (Log tab and dashboard Events panel).

Make the engine pure and testable: `fn evaluate(&mut self, snapshot: &Snapshot, now: AppInstant) -> Vec<Delivery>`. Here `Delivery { key, priority, text_builder_input }`. No I/O inside the engine.

---

## 8. Notification channels (`notify/`)

The notifier receives each `Delivery` and fans it out to the enabled channels. Every external command uses `tokio::process::Command` with **args passed directly** (never `sh -c`), and with `stdin/stdout/stderr = Stdio::null()` so nothing writes over the TUI. Commands are never awaited on the UI task.

- **Sound**: `afplay /System/Library/Sounds/<sound>` (default `Glass.aiff`).
- **Banner**: `osascript -e 'display notification "<msg>" with title "bbq-watch"'`. Escape `"` and `\` in the message.
- **ntfy**: `POST https://ntfy.sh/<topic>` with the message as the body. Add header `Title: bbq-watch`, and `Priority: urgent` for Urgent alerts or `default` otherwise. Use a timeout of 10 s. On failure, log it and continue.
- **Terminal bell**: write `\x07` through the ratatui backend (off by default).
- **Speech**: see §8.1.

### 8.1 Speech queue (`notify/speech.rs`)
A single worker task owns speech:

```rust
struct SpeechItem { key: AlertKey, priority: Priority }
```
- **Queue**: two FIFO lanes, Urgent and Normal. Urgent items are always taken first.
- **Dedupe**: if an item with the same key is already queued, don't add another.
- **Drop stale items**: just before speaking an item, check with the alert engine (via a shared `Arc<Mutex<ActiveAlerts>>` snapshot, or a query channel) that its key is still active and unacknowledged. If not, drop it.
- **Build the text when it's spoken**, not when the alert fires, so the numbers are current. Use the probe label and rounded whole numbers in the display unit. Examples:
  - TargetReached: "Pork butt is at 195 degrees. It's ready."
  - PreAlert: "Brisket point is at 198 degrees, 5 degrees from target."
  - BelowLow (pit): "Warning. Pit has dropped to 210 degrees."
  - ProbeUnplugged: "Brisket flat probe is disconnected."
  - BluetoothLost: "Lost connection to the thermometer."
  - StallStarted: "Brisket point has stalled at 150 degrees."
  - ServeLate: "Heads up. The cook is now estimated to finish after 7 30."
- **Speak**: `say [-v <voice>] [-r <rate>] [-a <device>] <text>`, then **`child.wait().await`**. `say` exits when it has finished speaking, so no duration estimate is needed. Pause 800 ms before the next item.
- **Pre-emption**: if an Urgent item arrives while a Normal item is being spoken, `kill()` the current child, put the Normal item back at the front of its lane and speak the Urgent one.
- **Quiet hours** (optional window, e.g. 23:00–07:00): only Urgent items are spoken.
- **Mode** `urgent_only`: Normal items are never spoken. Other channels still deliver them.
- **Periodic status** (default off; every N min): when the queue is idle, speak a summary like "Brisket point 162, flat 160, pork 178. Pit steady at 250." This is a Normal item with a special key, and it's never queued twice.
- If `say` isn't on PATH (Linux), disable speech and log a warning once. Optionally fall back to `spd-say` or `espeak` if found.
- Settings has a **"Preview voice"** action that queues a test phrase. "Test alarm" sends a fake Urgent delivery through every channel.

---

## 9. Configuration (`config.rs`)

Path: `directories::ProjectDirs::from("", "", "bbq-watch").config_dir()/config.toml` (on macOS, `~/Library/Application Support/bbq-watch/config.toml`), or the `--config` path. Create it with defaults if it's missing. Every field uses `#[serde(default)]`, so old configs keep loading. Save atomically (write a temp file, then rename).

```toml
[display]
units = "F"                 # "F" | "C"
chart_window = "whole"      # "whole" | "6h" | "2h" | "30m"
table_interval_min = 5      # 1 | 5 | 15
refresh_ms = 1000
theme = "tokyo-night"       # only this theme required in v1
show_target_lines = true
show_pit_range_lines = true
clock_24h = true
compact_cards = false

[[probes]]                  # exactly 4 entries, index = probe number - 1
label = "Brisket Point"
preset = "brisket"
target_c = 95.0             # null for pit preset
range_c = [107.2, 135.0]    # pit only: [min, max]
alert_low_c = null
alert_high_c = null
pre_alert_c = 2.8           # 5°F; null = off
color = "red"               # red|orange|green|blue|purple|cyan|yellow|white
show_on_chart = true
# defaults for P2..P4: "Brisket Flat"/brisket/orange, "Pork Butt"/pork_pulled/green, "Pit"/pit/blue

[alarms]
sound = true
sound_file = "Glass.aiff"
banner = true
ntfy = false
ntfy_topic = ""             # generate "bbq-watch-<4 random hex>" on first run
bell = false
repeat_s = 90
snooze_min = 10
unplugged_after_s = 30
bt_lost_after_min = 2
alert_on_stall = true

[speech]
enabled = true
voice = ""                  # empty = system default
rate_wpm = 180
audio_device = ""           # empty = default output
urgent_only = false
quiet_hours = ""            # e.g. "23:00-07:00"; empty = off
status_every_min = 0        # 0 = off

[cook]
eta_method = "stall_aware"  # "stall_aware" | "linear"
rate_window_min = 30
stall_rate_f_per_h = 3.0
stall_minutes = 20
stall_min_c = 62.8          # 145°F
stall_max_c = 79.4          # 175°F
rest_min = 60
want_to_eat_at = ""         # "HH:MM" or empty
wrapped_resets_stall = true
pause_eta_on_lid_open = false

[device]
peripheral_id = ""          # CoreBluetooth UUID, saved after first connect
display_name = ""           # user rename; shown instead of advertised name
auto_reconnect = true
battery_check_min = 5
device_screen_units = "match"   # "match" | "F" | "C"

[data]
csv_enabled = true
csv_dir = ""                # empty = ~/bbq-watch
sample_interval_s = 10      # 1 | 10 | 60
keep_awake = true
```

**Presets** (choosing one sets the target; "custom" leaves it alone):

| key | label | target |
|---|---|---|
| brisket | Brisket · probe-tender | 203°F |
| beef_ribs | Beef ribs | 203°F |
| pork_pulled | Pork shoulder · pulled | 195°F |
| pork_sliced | Pork shoulder · sliced | 185°F |
| pork_ribs | Pork ribs | 195°F |
| pork_loin | Pork loin | 145°F |
| chicken | Chicken breast / whole | 165°F |
| chicken_thigh | Chicken thighs | 175°F |
| turkey_breast | Turkey breast | 160°F |
| beef_mr | Beef · medium-rare | 130°F |
| salmon | Salmon | 140°F |
| pit | Pit / ambient | range 225–275°F, default alert_low 215°F, alert_high 290°F |
| custom | Custom | — |

---

## 10. Other services

- **CSV logger** (`logger.rs`): one file per cook, `<csv_dir>/cook-YYYY-MM-DD-HHMM.csv`, created on the first reading. Columns: `timestamp,elapsed_s,p1_c,p2_c,p3_c,p4_c,p1_f,p2_f,p3_f,p4_f,battery_pct,event`, with an empty cell for unplugged probes. Write every `sample_interval_s`, and also write a row whenever an event is logged (an alert fire or clear, wrapped, connect or disconnect). Flush after every row.
- **Keep awake** (`caffeinate.rs`): if `keep_awake` is on, spawn `caffeinate -dimsu -w <own pid>`, so it exits automatically when bbq-watch does. Kill it if the setting is turned off.
- **Tracing**: log to `<data_dir>/bbq-watch.log` (rolling daily). Never use `println!` after the TUI starts.

---

## 11. Application architecture

```
           ┌──────────────┐  DeviceEvent  ┌──────────────────────────────┐
 BLE/Demo ─┤ device task  ├──────────────▶│  App loop (single owner of   │
           └──────────────┘◀──────────────┤  all state; tokio::select!)  │
                         DeviceCommand    │  - crossterm EventStream     │
                                          │  - tick interval (refresh)   │
            ┌───────────────┐  Delivery   │  - device events             │
            │ notifier/     │◀────────────┤  - analysis + alert engine   │
            │ speech worker │             │  - draw on every tick/event  │
            └───────────────┘             └──────────────────────────────┘
```
- One `App` struct owns all state, so no mutex is needed except the small shared active-alerts snapshot that the speech worker reads.
- Use `ratatui::init()` / `ratatui::restore()`. `init()` installs a panic hook that restores the terminal; don't add another (§0.3).
- Quitting with `q` asks for confirmation ("Quit and end cook? y/n") if any probe is connected. On exit: restore the terminal, flush the CSV, kill `caffeinate` and send `DeviceCommand::Shutdown`.
- Use a minimum terminal size of 100×30. If the terminal is smaller, draw only a centered message "Enlarge terminal to at least 100×30".

---

## 12. UI specification

**Theme (Tokyo Night)**: bg `#1a1b26`, fg `#c0caf5`, border/dim `#565f89`, muted `#7a83a8`, accent `#ff9e64`, yellow `#e0af68`, green `#9ece6a`, red `#f7768e`, blue `#7aa2f7`, selected-row bg `#2f3549`, gauge track `#292e42`. Probe colors come from config (red `#f7768e`, orange `#ff9e64`, green `#9ece6a`, blue `#7aa2f7`, purple `#bb9af7`, cyan `#7dcfff`, yellow `#e0af68`, white `#c0caf5`). Use rounded borders (`BorderType::Rounded`) everywhere.

### 12.1 Frame (all tabs)
- **Header** (3 rows, bordered, title " bbq-watch " in accent):
  - Left side: a `Tabs` widget with ` 1 Dashboard │ 2 Table │ 3 Alarms │ 4 Log │ 5 Settings `. The selected tab is bold, with bg accent and fg background.
  - The Alarms tab title shows a count when alerts are active, e.g. `3 Alarms (2)`, in red.
  - Right side, right-aligned: `● <device name> │ <rssi> dBm │ batt <n>% │ cook <elapsed> │ HH:MM:SS`. The dot is green when connected, yellow while scanning and red when disconnected. The battery is red below 20%.
- **Footer** (1 row): key hints for the current tab, each rendered as a ` key ` chip (bg muted, fg background, bold) followed by a muted description. On the right, in dim text: `logging → <csv file name>`, or `demo ×<speed>` in demo mode.
- An **alert banner** appears while any alert is active and unacknowledged. It is drawn as a 1-row bar directly under the header, red bg for Urgent and yellow for Normal: `⚠ Pit dropped to 210°F  ·  x ack  z snooze`. Show the highest-priority, most recent alert.
- **Global keys**: `q` quit, `←/→` or `1`–`5` switch tabs, `x` ack all, `z` snooze all, `?` help popup listing all keys.

### 12.2 Dashboard
Layout (top to bottom): header; a probe cards row (height 8, 4 equal columns); a main area (fills the rest), split horizontally into the chart (about 70%) and a right column (about 30%), with the right column split vertically into the ETA panel (about 60%) and the Events panel (about 40%); and the footer.

**Probe card (meat)**, titled ` P1 · <label> ` in the probe color:
```
 150.4°F                 ▲ +1.8°F/h       <- temp bold in probe color; rate green if >5/h, yellow if stalled, red if negative
 target 203°F               STALL         <- STALL badge (yellow bg) if stalled; WRAPPED badge (blue) if wrapped; DONE badge (green) if done
                                          
 [██████████████████68%░░░░░░░░░]          <- Gauge: (c - start_temp)/(target - start_temp), start_temp = first reading this cook
 ETA 5h 10m                 ≈ 23:25 *     <- * if stall-adjusted
```
**Probe card (pit)**: the temp; `◆ steady ±N°F` (the max deviation over 10 min), green if within range; `range 225–275°F` with an `IN RANGE`/`LOW`/`HIGH` badge; and a horizontal track from (min−50) to (max+50) with the in-range segment shaded and a `█` marker at the current value, labelled underneath with min−50, the midpoint and max+50.
**Unplugged probe**: the temp shows `--` and the card shows "probe not connected" in dim text. A card with `show_on_chart = false` still shows.

`↑/↓` or `Tab` on the dashboard selects a card (highlighted border in accent), and `w` toggles "wrapped" on the selected meat probe.

**Chart**: a ratatui `Chart`, block title ` Temperature °F · since HH:MM ` (or ` · last 2h `, following `chart_window`).
- One `Dataset` per visible probe: `Marker::Braille`, `GraphType::Line`, the probe color and the label as its name.
- Target lines: an extra 2-point dataset per distinct meat target, drawn with `Marker::Dot` in a dimmed probe color and with no legend name. Pit range lines at min and max are dim blue when `show_pit_range_lines` is on.
- Y axis is 0 to (max(maximum value, maximum target) + 25) rounded up to 50, with labels every 50.
- X axis runs from the window start to now, with 5 evenly spaced `HH:MM` labels.
- The legend is in the bottom-right (`legend_position(Some(LegendPosition::BottomRight))`).
- Downsample each dataset to at most 2 points per chart column.

**ETA panel**, titled ` Estimated completion `. Meat probes are sorted by ETA, ascending, with 3 rows each:
```
● P3 Pork Butt                21:05       <- dot + bold label; time right-aligned in probe color, * if stall-adjusted
  165.2 → 195°F  +10.6°F/h   2h 50m
(blank)
```
Then a separator line, then `All probes done ≈ 23:40` and `+ 1h rest → serve ≈ 00:40` (red if later than want_to_eat_at). If any ETA is stall-adjusted, finish with a dim footnote: `* stall-adjusted. Straight-line fit gives <linear ETA> for <probes>.` If one probe is stalled and not wrapped, also add: `Wrap now → est. −<delta>`, which is the ETA difference if it were wrapped.

**Events panel**, titled ` Events `: the newest events first, as `HH:MM  <icon>  <message>`. Icons: `✓` green (milestones, target reached, connected), `⚠` yellow (stall, lid-open detection, normal alerts), `✖` red (urgent alerts), `•` muted (info). Log these milestones automatically: cook started, pit reached range min, each meat probe passing every 20°F multiple from 100°F, stall started/ended, lid open (pit drop >25°F in 3 min) and recovered.

### 12.3 Table
- A full-width `Table` block titled ` Readings · <N>-min intervals · newest first `, with ` <rows> rows · row <i> of <rows> ` on the right of the top border.
- Columns: `Time | P1 <label> | P2 … | P3 … | P4 … | P1 Δ | P2 Δ | P3 Δ | P4 Δ | Status`. Temperatures are formatted `{:>7.1}°F` in the probe color. Deltas are the change over one interval, `{:+.1}`: green if > +0.3, red if < −0.3, dim otherwise, and never shown as `-0.0`.
- Status shows any event in that interval, otherwise `stall` (dim) if any probe was stalled.
- The highlighted row has bg `#2f3549`, symbol `▶ ` in accent. Add a `Scrollbar` on the right edge.
- Below the table is a row of 4 blocks (height 8), one per probe, each containing a `Sparkline` of the whole cook in the probe color (scale 0–300°F, or the °C equivalent), with `min N · max N · now N` underneath.
- Keys: `↑/↓` scroll, `PgUp/PgDn`, `Home/End`, `i` cycles the interval (1/5/15 min), `e` exports the whole history to a CSV in csv_dir and logs the path as an event.

### 12.4 Alarms
- A table of active alerts: `Priority | Probe | Alert | Since | Last spoken | Status (active/acked/snoozed until HH:MM)`.
- Below it, a list of alerts cleared in the last hour, in dim text.
- Keys: `↑/↓` select, `Enter` ack the selected alert, `x` ack all, `z` snooze all, `t` test alarm.

### 12.5 Log
- A scrollable list of every event this cook (the same items as the Events panel, but complete, with full timestamps).
- Keys: `↑/↓/PgUp/PgDn/Home/End`.

### 12.6 Settings
Layout: a **Probes** section (full width, height 10). Below it, a 2×2 grid of sections: **Alarms & notifications** (top left), **Cook & estimates** (top right), **Display** (bottom left) and **Device & data** (bottom right). Add a **Speech** section, either as a fifth block, or by splitting Alarms & notifications vertically if space requires. On terminals shorter than 40 rows, make the grid area scroll.

The section with focus has an accent-coloured border title. Navigation: `Tab`/`Shift-Tab` move between sections, `↑/↓` move between fields, `←/→` (or `+/-`) adjust numbers and choices, `Space` toggles checkboxes, `Enter` edits text or time fields and activates buttons, `s` saves, `Esc` discards. While any change is unsaved, show ` ● unsaved changes ` in yellow on the right of the Probes border. Changes apply only on save, except Preview voice and Test alarm.

Widgets (all drawn manually with `Span`s):
- checkbox `[x]`/`[ ]`, green when checked;
- radio `(•)`/`( )`, accent when selected;
- segmented control: a ` opt ` chip for each option, with the selected option on bg accent and the others on bg `#292e42`;
- number spinner `◀ 30 ▶ unit`, with the value bold on `#292e42`;
- text input: the value on `#3b4261` with a `▏` cursor in accent while editing;
- button: a ` Label ` chip.

Each field type must have a min/max/step: for example, repeat 15–600 s in steps of 15; snooze 1–60 min; rate window 10–120 min; stall rate 1–6 in steps of 0.5; rest 0–240 min in steps of 15.

**Probes section**: a table with columns `# | Label | Preset | Target | Alert low | Alert high | Pre-alert | Color | Chart`.
- `↑/↓` pick the row and `←/→` pick the column.
- `Enter` on Label edits it (max 18 characters).
- `p`, or `Enter` on Preset, opens a popup list of presets. Draw it with the `Clear` widget plus a bordered `List`. Choosing one sets the target and range.
- `+/-` on Target or an alert column adjusts it by 1° (5° with Shift). `Delete` on an alert column sets it to `—`.
- `Enter` on Color cycles the color, and `Space` on Chart toggles it.
- Below the table, a hint line: `Enter edit label   p choose preset   +/- adjust target (1°F, Shift 5°F)   Labels are saved per-probe and reused next cook`.

**Alarms & notifications fields**:
- toggles: sound on this Mac (sound file), macOS banner, push via ntfy, terminal bell, alert on stall;
- the ntfy topic as text;
- spinners: repeat every N s, snooze N min, probe unplugged after N s, Bluetooth lost after N min;
- a `Test alarm` button.

**Speech fields**:
- enabled;
- voice, a choice list filled at startup from `say -v '?'` (parse the first column; include "System default");
- rate in wpm (90–360);
- audio output, a choice list from `say -a '?'` plus "Default";
- urgent only;
- quiet hours (a text field in `HH:MM-HH:MM` form, validated);
- spoken status every N min (0 = off);
- a `Preview voice` button.

**Cook & estimates fields**: ETA method (radio), rate window, stall detection (rate, minutes, min temp, max temp), rest time, want-to-eat-at (time text field, validated `HH:MM`, empty allowed), wrapped resets stall, auto-pause ETA while lid open.

**Display fields**: units (segmented °F/°C), chart window (segmented), table interval (segmented), refresh (spinner 250–5000 ms in steps of 250), theme (choice with a single option), show target lines, show pit range lines, 24-hour clock, compact cards.

**Device & data fields**:
- thermometer line `● <name> <shortened peripheral id>` with buttons Scan, Forget and Rename… (Rename edits `display_name`);
- auto-reconnect;
- battery check minutes;
- device screen units (segmented Match app/°F/°C);
- log to CSV, with the directory as text;
- sample every (segmented 1s/10s/60s);
- keep Mac awake;
- a read-only line `Config: <path>`.

---

## 13. Error handling and macOS notes
- On the first run, macOS asks for Bluetooth permission for the terminal app. If the scan finds no adapter, or the permission is denied, show a centered message on the dashboard: "Bluetooth unavailable — allow your terminal in System Settings → Privacy & Security → Bluetooth". Keep retrying every 10 s.
- The Inkbird phone app must not be connected, because the thermometer accepts only one connection. Show this hint while scanning: "Scanning… (close the Inkbird phone app if it's connected)".
- An external command failing (say, afplay, osascript, caffeinate) must never crash the app. Log it, and show it once as a Log-tab event.
- No `unwrap`/`expect`/`panic!` outside tests (§0.3). Use `anyhow` with context at the edges and `thiserror` in typed modules.
- **Linux compile**: btleplug's Linux backend needs `libdbus-1-dev` and `pkg-config` installed to build. Nothing in `bbq-watch` may be macOS-only at *compile* time. `say`, `afplay`, `osascript` and `caffeinate` are run as external programs and can fail at runtime, so `cargo build --workspace` must keep working on a Linux CI runner with those two packages installed. Don't use `#[cfg(target_os = "macos")]` to remove modules. Detecting missing commands at runtime is enough.

---

## 14. Build order (implement in this sequence; keep `cargo build -p bbq-watch` and `cargo clippy -p bbq-watch --all-targets -- -D warnings` green after each step)
1. Workspace integration (§0): add the member and the `[workspace.dependencies]` entries to the root `Cargo.toml`, and create `crates/bbq-watch` with the manifest from §0.2, `lib.rs`, a thin `main.rs` and the README. Check that `cargo build --workspace` passes and that `git status` shows changes only to the root `Cargo.toml`, `Cargo.lock` and `crates/bbq-watch/`. Then add the CLI, AppClock, and config load/save with defaults and presets.
2. `ibbq.rs` pure parse functions plus unit tests.
3. Demo device, and a minimal App loop that prints probe temps in a basic ratatui frame (header, tabs, footer).
4. History, rate, stall and ETA (`analysis.rs`) plus unit tests.
5. The full Dashboard (cards, chart, ETA panel, events).
6. The Table tab, then the Log tab.
7. The alert engine plus unit tests, then the Alarms tab and the banner.
8. Notifier channels; the speech queue plus unit tests of the queue logic (abstract the "speaker" behind a trait so tests can use a fake that records calls).
9. The Settings tab with full editing and saving.
10. CSV logger and caffeinate.
11. The BLE device implementation (scan, connect, login, subscribe, battery, RSSI, reconnect, Forget).
12. Polish: help popup, small-terminal message, quit confirmation, README. Then run the full gate in §15.

---

## 15. Acceptance criteria

**Repository gate** (all must pass from the workspace root):
```
cargo fmt --all -- --check
cargo clippy -p bbq-watch --all-targets -- -D warnings
cargo test -p bbq-watch
cargo build --workspace
cargo tree -p bbq-watch -d      # must show no duplicate crossterm or ratatui
git status --porcelain          # only Cargo.toml, Cargo.lock and crates/bbq-watch/** changed
```
Also check that there's no `unsafe`, and no `unwrap(`/`expect(`/`println!`/`eprintln!`/`dbg!`/`todo!` outside `#[cfg(test)]` modules in `crates/bbq-watch/src`.

**Automated (`cargo test`)**
- `parse_realtime(&[0xE0,0x05, 0xF6,0xFF, 0x2C,0x06, 0x9C,0x09])` returns `[Some(150.4), None, Some(158.0), Some(246.0)]` (°C tenths → °C: 0x05E0 = 1504 → 150.4).
- `parse_battery(&[0x24, 0x10,0x0E, 0x00,0x00])` returns `Some(55)` (3600/6550).
- Regression rate on a synthetic line rising 10°F/h over 30 minutes is 10 ± 0.1 °F/h.
- Stall detection: a series flat at 150°F for 25 minutes after rising is stalled. The same series flat for only 15 minutes is not.
- Stall-aware ETA with stall_started 100 min ago, target 203°F, current 150.4°F and pre-stall rate 12°F/h is 80 min + 263 min = 343 min ± 1.
- Alert engine with a fake clock: a TargetReached alert is delivered at t=0, then at t=90 s, t=270 s, t=570 s and t=870 s; an ack stops the repeats; temperature dropping 2°F below target clears it; a second different alert raised 10 s after the first is delivered immediately, because cooldowns are per key.
- Speech queue with a fake speaker: duplicates are collapsed; Urgent goes before Normal; an item whose alert cleared before its turn is dropped; an Urgent item pre-empts (kills) a Normal item that's being spoken; with urgent_only, Normal items are never spoken.

**Manual (`cargo run -p bbq-watch -- --demo --speed 60`)**
- All 5 tabs render without panics at 132×40 and at 100×30. Resizing never panics.
- Within about 3 minutes of real time (about 3 simulated hours), you see the pit lid-open dip on the chart and an event, a stall detected on P1/P2 with STALL badges and `*` ETAs, and P3's ETA counting down.
- Pressing `#` (unplug P3) gives a ProbeUnplugged alert after 30 simulated seconds: the banner shows, and `say` speaks "Pork butt probe is disconnected." Pressing `x` stops the repeats.
- Editing P3's label in Settings and saving updates the card title, chart legend, table header and spoken alerts, and the label survives a restart.
- Switching units to °C updates every displayed value, and targets stay the same physical temperature.
- A CSV file appears in `~/bbq-watch/` with a header and rows.

**Manual with hardware**: `cargo run -p bbq-watch --release` finds the IBT-4XS, shows 4 probe values that match the device screen (±0.2°), shows a battery percentage, and reconnects automatically after the thermometer is switched off and on again.
