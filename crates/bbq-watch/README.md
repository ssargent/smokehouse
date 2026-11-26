# bbq-watch

Terminal dashboard and alerting for **Inkbird IBT-4XS** iBBQ Bluetooth
thermometers, written in Rust. It shows live probe temperatures, a history
chart and table, and stall-aware estimated finish times, and it raises sound,
macOS notification, [ntfy](https://ntfy.sh) push and spoken (`say`) alerts.

## Running

From the workspace root:

```sh
cargo run -p bbq-watch --release              # real thermometer (BLE)
cargo run -p bbq-watch -- --demo --speed 60   # simulated thermometer, 1 s = 1 min
```

Flags:

| Flag | Meaning |
|---|---|
| `--demo` | Use the simulated thermometer instead of BLE. |
| `--speed N` | Demo time multiplier (default `1`). `--speed 60` makes one real second equal one simulated minute. |
| `--config <path>` | Use a different configuration file. |
| `--no-speech` | Force spoken alerts off for this run. |

In demo mode, `!` `@` `#` `$` (Shift+1–4) toggle probes P1–P4 as *unplugged*, so
you can exercise the disconnect alerts without hardware.

## macOS Bluetooth permission

On the first real run, macOS asks for Bluetooth permission for the terminal
application. Allow it in **System Settings → Privacy & Security → Bluetooth**.
If the scan finds no adapter or permission is denied, the dashboard shows a
reminder and keeps retrying every 10 seconds.

The thermometer accepts only one connection, so **close the Inkbird phone app**
before connecting. While scanning, the dashboard shows a hint to that effect.

## Where files live

| Path | Contents |
|---|---|
| `~/Library/Application Support/bbq-watch/config.toml` | Settings (TOML), created with defaults on first run. |
| `~/Library/Application Support/bbq-watch/bbq-watch.log` | Rolling diagnostic log (daily). |
| `~/bbq-watch/cook-YYYY-MM-DD-HHMM.csv` | One CSV per cook (change the directory in Settings). |

Settings can also be pointed at another file with `--config`.

## Cook presets

Presets are stored in the config file under `[[presets]]` (id, name, target in
°F, and the pit range and alerts), so they can be edited by hand or from the
Settings tab. Each probe's `preset` field holds the id of the preset it uses.

In the Settings tab (`5`), press `p` on the Probes table (or `Enter` on a
probe's `Preset` column) to open the preset list:

| Key | Action |
|---|---|
| `↑`/`↓` | Choose a preset. |
| `Enter` | Apply it to the selected probe. |
| `e` | Edit it: name, target, pit flag, pit range and pit alerts. |
| `n` | Add a preset after the highlighted one. |
| `d` | Delete the highlighted preset (probes using it fall back to a neighbour). |
| `Esc` | Close the list. |

Preset edits, like every other setting, are written to the config file with `s`.
