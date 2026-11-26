//! Notification fan-out: sound, macOS banner, ntfy push and speech.
//!
//! Every external command runs on its own task with null stdio, so nothing can
//! write over the TUI, and a failing command is logged, never fatal.

pub mod banner;
pub mod ntfy;
pub mod sound;
pub mod speech;

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use std::sync::Mutex;
use tokio::sync::mpsc;

use crate::alerts::{ActiveAlerts, Delivery, Priority};
use crate::clock::AppClock;
use crate::config::Settings;
use speech::{SpeechHandle, SpeechItem, SpeechKey};

/// A failure in one of the notifier's external commands.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NotifyError(pub String);

impl NotifyError {
    /// Build from anything printable.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Messages the notifier task handles.
#[derive(Debug)]
pub enum NotifyMsg {
    /// Settings changed.
    Settings(Box<Settings>),
    /// Deliver one alert.
    Delivery(Box<Delivery>),
    /// Send a fake urgent delivery through every channel.
    Test,
    /// Speak the periodic status summary.
    Status,
    /// Speak the voice-preview phrase.
    PreviewVoice,
    /// Stop the notifier and the speech worker.
    Shutdown,
}

/// Handle for sending work to the notifier task.
#[derive(Clone, Debug)]
pub struct Notifier {
    tx: mpsc::Sender<NotifyMsg>,
}

impl Notifier {
    /// Deliver an alert.
    pub fn deliver(&self, delivery: Delivery) {
        if self
            .tx
            .try_send(NotifyMsg::Delivery(Box::new(delivery)))
            .is_err()
        {
            tracing::warn!("notifier channel full; dropped an alert");
        }
    }

    /// Replace the settings the notifier uses.
    pub fn update_settings(&self, settings: &Settings) {
        let _ = self
            .tx
            .try_send(NotifyMsg::Settings(Box::new(settings.clone())));
    }

    /// Send a fake urgent delivery through every channel.
    pub fn test(&self) {
        let _ = self.tx.try_send(NotifyMsg::Test);
    }

    /// Queue the voice preview phrase.
    pub fn preview_voice(&self) {
        let _ = self.tx.try_send(NotifyMsg::PreviewVoice);
    }

    /// Queue the periodic status summary.
    pub fn status(&self) {
        let _ = self.tx.try_send(NotifyMsg::Status);
    }

    /// Stop the notifier.
    pub fn shutdown(&self) {
        let _ = self.tx.try_send(NotifyMsg::Shutdown);
    }
}

/// Spawn the notifier and speech tasks.
#[must_use]
pub fn spawn(
    clock: AppClock,
    shared: Arc<Mutex<ActiveAlerts>>,
    settings: &Settings,
    no_speech: bool,
    errors: mpsc::Sender<String>,
) -> Notifier {
    let (tx, rx) = mpsc::channel(64);
    let speech = speech::spawn(clock, Arc::clone(&shared), settings, no_speech);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok();
    let settings = settings.clone();
    tokio::spawn(async move {
        run(rx, speech, shared, client, settings, errors).await;
    });
    Notifier { tx }
}

async fn run(
    mut rx: mpsc::Receiver<NotifyMsg>,
    speech: SpeechHandle,
    shared: Arc<Mutex<ActiveAlerts>>,
    client: Option<reqwest::Client>,
    mut settings: Settings,
    errors: mpsc::Sender<String>,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            NotifyMsg::Settings(new) => {
                settings = *new;
                speech.update_settings(&settings);
            }
            NotifyMsg::Delivery(delivery) => {
                dispatch(
                    &delivery,
                    &settings,
                    &speech,
                    &shared,
                    client.as_ref(),
                    &errors,
                )
                .await;
            }
            NotifyMsg::Test => {
                dispatch_test(&settings, &speech, client.as_ref(), &errors).await;
            }
            NotifyMsg::Status => {
                speech.item(SpeechItem {
                    key: SpeechKey::Status,
                    priority: Priority::Normal,
                });
            }
            NotifyMsg::PreviewVoice => {
                speech.item(SpeechItem {
                    key: SpeechKey::Preview,
                    priority: Priority::Urgent,
                });
            }
            NotifyMsg::Shutdown => {
                speech.shutdown();
                break;
            }
        }
    }
}

async fn dispatch(
    delivery: &Delivery,
    settings: &Settings,
    speech: &SpeechHandle,
    shared: &Arc<Mutex<ActiveAlerts>>,
    client: Option<&reqwest::Client>,
    errors: &mpsc::Sender<String>,
) {
    let message = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .build_text(&delivery.key);
    let urgent = delivery.priority == Priority::Urgent;
    fan_out(settings, &message, urgent, client, errors);
    speech.item(SpeechItem {
        key: SpeechKey::Alert(delivery.key),
        priority: delivery.priority,
    });
}

async fn dispatch_test(
    settings: &Settings,
    speech: &SpeechHandle,
    client: Option<&reqwest::Client>,
    errors: &mpsc::Sender<String>,
) {
    let message = String::from("Test alarm from bbq-watch.");
    fan_out(settings, &message, true, client, errors);
    speech.item(SpeechItem {
        key: SpeechKey::Test,
        priority: Priority::Urgent,
    });
}

fn fan_out(
    settings: &Settings,
    message: &str,
    urgent: bool,
    client: Option<&reqwest::Client>,
    errors: &mpsc::Sender<String>,
) {
    if settings.alarms.sound {
        let file = settings.alarms.sound_file.clone();
        spawn_logged("sound", settings_errors(errors), async move {
            sound::play(&file).await
        });
    }
    if settings.alarms.banner {
        let text = message.to_string();
        spawn_logged("banner", settings_errors(errors), async move {
            banner::show(&text).await
        });
    }
    if settings.alarms.ntfy
        && !settings.alarms.ntfy_topic.trim().is_empty()
        && let Some(client) = client
    {
        let client = client.clone();
        let text = message.to_string();
        let topic = settings.alarms.ntfy_topic.clone();
        spawn_logged("ntfy", settings_errors(errors), async move {
            ntfy::push(&client, &topic, &text, urgent).await
        });
    }
}

fn settings_errors(errors: &mpsc::Sender<String>) -> mpsc::Sender<String> {
    // Sender is already a cheap handle; spawn_logged can own a clone.
    errors.clone()
}

fn spawn_logged<F>(name: &'static str, errors: mpsc::Sender<String>, fut: F)
where
    F: Future<Output = Result<(), NotifyError>> + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(err) = fut.await {
            tracing::warn!("{name} failed: {err}");
            let _ = errors.try_send(format!("{name} failed: {err}"));
        }
    });
}

/// Run a program with null stdio and report whether it exited cleanly.
pub(crate) async fn run_command(program: &str, args: &[&str]) -> Result<(), NotifyError> {
    let status = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|err| NotifyError::new(format!("{program}: {err}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(NotifyError::new(format!("{program} exited with {status}")))
    }
}

/// Whether a `say`-family command exists on `PATH`.
#[must_use]
pub fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}
