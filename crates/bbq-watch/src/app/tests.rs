#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Build a demo-mode app on a throwaway config path.
pub(super) async fn harness() -> (App, PathBuf) {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("bbq-watch-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    let settings = config::load(&path).unwrap();
    let cli = Cli {
        demo: true,
        speed: 1.0,
        config: Some(path.clone()),
        no_speech: true,
    };
    let clock = AppClock::new(1.0);
    let (app, _device_rx, _error_rx) = build(cli, settings, clock, path.clone()).await;
    (app, path)
}

pub(super) async fn press(app: &mut App, code: KeyCode) {
    app.on_input(CrosstermEvent::Key(KeyEvent::new(
        code,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )));
}

pub(super) async fn press_mod(
    app: &mut App,
    code: KeyCode,
    modifiers: ratatui::crossterm::event::KeyModifiers,
) {
    app.on_input(CrosstermEvent::Key(KeyEvent::new(code, modifiers)));
}

pub(super) async fn type_str(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, KeyCode::Char(character)).await;
    }
}

#[allow(dead_code)]
pub(super) fn render(app: &App) -> String {
    let backend = TestBackend::new(132, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn cleanup(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

#[tokio::test]
async fn esc_never_discards_settings() {
    let (mut app, path) = harness().await;
    press(&mut app, KeyCode::Char('5')).await;
    // Probes section: Enter opens the probe form, Enter edits the label.
    press(&mut app, KeyCode::Enter).await;
    press(&mut app, KeyCode::Enter).await;
    press_mod(
        &mut app,
        KeyCode::Char('u'),
        ratatui::crossterm::event::KeyModifiers::CONTROL,
    )
    .await;
    type_str(&mut app, "Pork Shoulder").await;
    press(&mut app, KeyCode::Enter).await;
    for _ in 0..3 {
        press(&mut app, KeyCode::Esc).await;
    }
    assert_eq!(app.settings.probes[0].label, "Pork Shoulder");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("Pork Shoulder"), "config not saved:\n{text}");
    cleanup(&path);
}

#[tokio::test]
async fn space_in_preset_list_does_not_toggle_pit() {
    let (mut app, path) = harness().await;
    press(&mut app, KeyCode::Char('5')).await;
    // Move from Probes to the Presets section.
    press(&mut app, KeyCode::Tab).await;
    press(&mut app, KeyCode::Char(' ')).await;
    assert!(!app.settings.presets[0].pit);
    assert!(!app.settings.is_pit(0));
    cleanup(&path);
}

#[test]
fn serve_late_compares_full_datetimes() {
    use chrono::TimeZone as _;
    let start = Local.with_ymd_and_hms(2026, 1, 1, 12, 30, 0).unwrap();
    let want = serve_deadline(start, 19, 30).unwrap();
    // Serving at 00:40 the next day is later than a 19:30 want.
    let serve = start + chrono::Duration::minutes(12 * 60 + 10);
    assert!(serve > want);
    // Serving at 17:30 the same day is not late.
    let early = start + chrono::Duration::minutes(5 * 60);
    assert!(early <= want);
}

#[tokio::test]
async fn disconnect_clears_temps_and_keeps_last_seen() {
    let (mut app, path) = harness().await;
    app.on_device_event(DeviceEvent::Readings {
        at: AppInstant::from_secs_f64(1.0),
        temps_c: [Some(50.0), Some(60.0), None, None],
    })
    .await;
    assert_eq!(app.probes[0].temp_c, Some(50.0));
    app.on_device_event(DeviceEvent::Disconnected {
        reason: String::from("test"),
    })
    .await;
    assert!(app.probes.iter().all(|probe| probe.temp_c.is_none()));
    assert_eq!(app.probes[0].last_seen_c, Some(50.0));
    cleanup(&path);
}

#[tokio::test]
async fn repeated_errors_are_rate_limited() {
    let (mut app, path) = harness().await;
    app.push_error_event("boom");
    app.push_error_event("boom");
    assert_eq!(app.events.len(), 1);
    // Pretend the five-minute window has passed.
    app.error_events
        .insert(String::from("boom"), AppInstant::from_secs_f64(-10_000.0));
    app.push_error_event("boom");
    assert_eq!(app.events.len(), 2);
    cleanup(&path);
}

#[tokio::test]
async fn every_setting_is_visible() {
    let (mut app, path) = harness().await;
    press(&mut app, KeyCode::Char('5')).await;
    for section in 2..7 {
        let labels = ui::settings::field_labels(section);
        assert!(!labels.is_empty(), "section {section} has no fields");
        for (index, label) in labels.iter().enumerate() {
            app.settings_ui.section = section;
            app.settings_ui.field = index;
            let text = render(&app);
            assert!(
                text.contains(label),
                "section {section} field {label:?} not visible"
            );
        }
    }
    cleanup(&path);
}

#[tokio::test]
async fn probe_and_preset_forms_open() {
    let (mut app, path) = harness().await;
    press(&mut app, KeyCode::Char('5')).await;
    press(&mut app, KeyCode::Enter).await;
    assert!(render(&app).contains("Edit probe P1"));
    press(&mut app, KeyCode::Esc).await;
    press(&mut app, KeyCode::Tab).await;
    press(&mut app, KeyCode::Enter).await;
    let text = render(&app);
    assert!(
        text.contains("Edit preset"),
        "preset form not open:\n{text}"
    );
    assert!(
        text.contains("Used by: P1, P2"),
        "used-by line missing:\n{text}"
    );
    cleanup(&path);
}

#[tokio::test]
async fn preset_form_uses_user_units() {
    let (mut app, path) = harness().await;
    app.settings.display.units = Unit::Celsius;
    press(&mut app, KeyCode::Char('5')).await;
    press(&mut app, KeyCode::Tab).await;
    press(&mut app, KeyCode::Enter).await;
    let text = render(&app);
    assert!(text.contains("95"), "target should show °C:\n{text}");
    press(&mut app, KeyCode::Down).await; // Type
    press(&mut app, KeyCode::Down).await; // Target
    let before = app.settings.presets[0].target_f.unwrap();
    press(&mut app, KeyCode::Right).await;
    let after = app.settings.presets[0].target_f.unwrap();
    assert!((after - before - 1.8).abs() < 0.01, "expected a 1°C step");
    cleanup(&path);
}
