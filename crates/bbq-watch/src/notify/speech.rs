//! Speech queue worker: prioritised lanes, dedupe, stale-drop and pre-emption.

use std::collections::VecDeque;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc};
use tokio::task::AbortHandle;

use crate::alerts::{ActiveAlerts, AlertKey, Priority};
use crate::clock::{AppClock, Clock};
use crate::config::{Settings, SpeechSettings, in_quiet_hours};

/// The future returned by a [`Speaker`]; it owns whatever it needs to run.
pub type SpeechFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;

/// Identifies a spoken item, for dedupe and stale-drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpeechKey {
    /// An alert phrase.
    Alert(AlertKey),
    /// The periodic status summary.
    Status,
    /// The voice-preview phrase.
    Preview,
    /// The test-alarm phrase.
    Test,
}

impl SpeechKey {
    /// The alert key, when this is an alert phrase.
    #[must_use]
    pub fn alert(self) -> Option<AlertKey> {
        match self {
            Self::Alert(key) => Some(key),
            _ => None,
        }
    }
}

/// One queued speech item.
#[derive(Clone, Copy, Debug)]
pub struct SpeechItem {
    /// What to speak.
    pub key: SpeechKey,
    /// Which lane it belongs to.
    pub priority: Priority,
}

/// Messages the speech worker handles.
#[derive(Debug)]
pub enum SpeechMsg {
    /// Queue an item.
    Item(SpeechItem),
    /// Replace the speech settings.
    Settings(Box<Settings>),
    /// Queue the voice preview.
    Preview,
    /// Stop the worker.
    Shutdown,
}

/// Abstraction over the speech command, so tests can substitute a fake.
pub trait Speaker: Send {
    /// Start speaking; the returned future completes when speech finishes.
    fn start(&mut self, text: &str, settings: &SpeechSettings) -> SpeechFuture;
}

/// Which command backs the speaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpeakerKind {
    Say,
    SpdSay,
    Espeak,
}

struct CommandSpeaker {
    program: String,
    kind: SpeakerKind,
}

impl Speaker for CommandSpeaker {
    fn start(&mut self, text: &str, settings: &SpeechSettings) -> SpeechFuture {
        let mut cmd = tokio::process::Command::new(&self.program);
        match self.kind {
            SpeakerKind::Say => {
                if !settings.voice.is_empty() {
                    cmd.arg("-v").arg(&settings.voice);
                }
                cmd.arg("-r").arg(settings.rate_wpm.to_string());
                if !settings.audio_device.is_empty() {
                    cmd.arg("-a").arg(&settings.audio_device);
                }
                cmd.arg(text);
            }
            SpeakerKind::SpdSay => {
                cmd.arg(text);
            }
            SpeakerKind::Espeak => {
                if !settings.voice.is_empty() {
                    cmd.arg("-v").arg(&settings.voice);
                }
                cmd.arg("-s").arg(settings.rate_wpm.to_string());
                cmd.arg(text);
            }
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let program = self.program.clone();
        Box::pin(async move {
            let mut child = cmd.spawn().map_err(|err| format!("{program}: {err}"))?;
            let status = child
                .wait()
                .await
                .map_err(|err| format!("{program}: {err}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("{program} exited with {status}"))
            }
        })
    }
}

/// Two FIFO lanes; urgent items always come first.
#[derive(Debug, Default)]
pub struct SpeechQueue {
    urgent: VecDeque<SpeechItem>,
    normal: VecDeque<SpeechItem>,
}

impl SpeechQueue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an item unless its key is already queued.
    pub fn push(&mut self, item: SpeechItem) {
        if self.contains(item.key) {
            return;
        }
        if item.priority == Priority::Urgent {
            self.urgent.push_back(item);
        } else {
            self.normal.push_back(item);
        }
    }

    /// Whether an item with this key is queued.
    #[must_use]
    pub fn contains(&self, key: SpeechKey) -> bool {
        self.urgent
            .iter()
            .chain(self.normal.iter())
            .any(|item| item.key == key)
    }

    /// Number of queued items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.urgent.len() + self.normal.len()
    }

    /// Whether nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.urgent.is_empty() && self.normal.is_empty()
    }

    /// Pop the next item: urgent lane first.
    pub fn take_next(&mut self) -> Option<SpeechItem> {
        self.urgent.pop_front().or_else(|| self.normal.pop_front())
    }

    /// Put an item back at the front of its lane (used by pre-emption).
    pub fn requeue_front(&mut self, item: SpeechItem) {
        if item.priority == Priority::Urgent {
            self.urgent.push_front(item);
        } else {
            self.normal.push_front(item);
        }
    }
}

/// Handle for sending work to the speech worker.
#[derive(Clone, Debug)]
pub struct SpeechHandle {
    tx: mpsc::Sender<SpeechMsg>,
}

impl SpeechHandle {
    /// Queue an item.
    pub fn item(&self, item: SpeechItem) {
        let _ = self.tx.try_send(SpeechMsg::Item(item));
    }

    /// Replace the speech settings.
    pub fn update_settings(&self, settings: &Settings) {
        let _ = self
            .tx
            .try_send(SpeechMsg::Settings(Box::new(settings.clone())));
    }

    /// Queue the voice preview.
    pub fn preview(&self) {
        let _ = self.tx.try_send(SpeechMsg::Preview);
    }

    /// Stop the worker.
    pub fn shutdown(&self) {
        let _ = self.tx.try_send(SpeechMsg::Shutdown);
    }
}

/// Spawn the speech worker.
#[must_use]
pub fn spawn(
    clock: Arc<AppClock>,
    shared: Arc<Mutex<ActiveAlerts>>,
    settings: &Settings,
    no_speech: bool,
) -> SpeechHandle {
    let (tx, mut rx) = mpsc::channel(64);
    match detect_speaker() {
        Some(mut speaker) => {
            let mut speech_settings = settings.speech.clone();
            tokio::spawn(async move {
                let clock_ref: &dyn Clock = clock.as_ref();
                run_worker(
                    speaker.as_mut(),
                    clock_ref,
                    &shared,
                    &mut speech_settings,
                    no_speech,
                    &mut rx,
                )
                .await;
            });
        }
        None => {
            tracing::warn!("no speech command found on PATH; spoken alerts are disabled");
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
        }
    }
    SpeechHandle { tx }
}

/// The worker loop: start items, pre-empt on urgent arrivals.
async fn run_worker(
    speaker: &mut dyn Speaker,
    clock: &dyn Clock,
    shared: &Arc<Mutex<ActiveAlerts>>,
    settings: &mut SpeechSettings,
    force_off: bool,
    rx: &mut mpsc::Receiver<SpeechMsg>,
) {
    let mut queue = SpeechQueue::new();
    let (done_tx, mut done_rx) = mpsc::channel::<()>(8);
    let mut current: Option<SpeechItem> = None;
    let mut abort_handle: Option<AbortHandle> = None;

    loop {
        if current.is_none()
            && let Some(item) = next_item(&mut queue, shared, settings, force_off, clock).await
        {
            let text = phrase(item, shared).await;
            if let Some(key) = item.key.alert() {
                shared.lock().await.mark_spoken(&key, clock.now());
            }
            let future = speaker.start(&text, settings);
            let done = done_tx.clone();
            let task = tokio::spawn(async move {
                let _ = future.await;
                let _ = done.send(()).await;
            });
            abort_handle = Some(task.abort_handle());
            current = Some(item);
        }

        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    None | Some(SpeechMsg::Shutdown) => break,
                    Some(SpeechMsg::Settings(new)) => *settings = new.speech,
                    Some(SpeechMsg::Preview) => queue.push(SpeechItem {
                        key: SpeechKey::Preview,
                        priority: Priority::Urgent,
                    }),
                    Some(SpeechMsg::Item(item)) => {
                        let duplicate = queue.contains(item.key)
                            || current.is_some_and(|queued| queued.key == item.key);
                        let preempt = item.priority == Priority::Urgent
                            && !duplicate
                            && current.is_some_and(|queued| queued.priority != Priority::Urgent);
                        if preempt {
                            if let Some(handle) = abort_handle.take() {
                                handle.abort();
                            }
                            while done_rx.try_recv().is_ok() {}
                            if let Some(queued) = current.take() {
                                queue.requeue_front(queued);
                            }
                        }
                        queue.push(item);
                    }
                }
            }
            _ = done_rx.recv() => {
                current = None;
                abort_handle = None;
                tokio::time::sleep(Duration::from_millis(800)).await;
            }
        }
    }
}

/// Take the next item that should actually be spoken, dropping stale ones.
async fn next_item(
    queue: &mut SpeechQueue,
    shared: &Arc<Mutex<ActiveAlerts>>,
    settings: &SpeechSettings,
    force_off: bool,
    clock: &dyn Clock,
) -> Option<SpeechItem> {
    while let Some(item) = queue.take_next() {
        if !allowed(item, settings, force_off, clock) {
            continue;
        }
        if let Some(key) = item.key.alert()
            && !shared.lock().await.is_active_unacked(&key)
        {
            continue;
        }
        return Some(item);
    }
    None
}

fn allowed(
    item: SpeechItem,
    settings: &SpeechSettings,
    force_off: bool,
    clock: &dyn Clock,
) -> bool {
    if force_off || !settings.enabled {
        return false;
    }
    let urgent = item.priority == Priority::Urgent;
    if settings.urgent_only && !urgent {
        return false;
    }
    if !urgent {
        let wall = clock.wall(clock.now());
        use chrono::Timelike as _;
        if in_quiet_hours(&settings.quiet_hours, wall.hour(), wall.minute()) {
            return false;
        }
    }
    true
}

async fn phrase(item: SpeechItem, shared: &Arc<Mutex<ActiveAlerts>>) -> String {
    match item.key {
        SpeechKey::Alert(key) => shared.lock().await.build_text(&key),
        SpeechKey::Status => shared.lock().await.build_status(),
        SpeechKey::Preview => String::from("This is the bbq-watch voice preview."),
        SpeechKey::Test => String::from("Test alarm from bbq-watch."),
    }
}

/// Pick the first available speech command.
fn detect_speaker() -> Option<Box<dyn Speaker>> {
    let candidates = [
        ("say", SpeakerKind::Say),
        ("spd-say", SpeakerKind::SpdSay),
        ("espeak", SpeakerKind::Espeak),
    ];
    for (program, kind) in candidates {
        if super::command_exists(program) {
            return Some(Box::new(CommandSpeaker {
                program: program.to_string(),
                kind,
            }));
        }
    }
    None
}

/// Voice names from `say -v '?'`, with `System default` first.
pub async fn list_voices() -> Vec<String> {
    let mut voices = vec![String::from("System default")];
    if let Ok(output) = tokio::process::Command::new("say")
        .arg("-v")
        .arg("?")
        .output()
        .await
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Some(name) = line.split_whitespace().next()
                && !voices.iter().any(|voice| voice == name)
            {
                voices.push(name.to_string());
            }
        }
    }
    voices
}

/// Audio output names from `say -a '?'`, with `Default` first.
pub async fn list_audio_devices() -> Vec<String> {
    let mut devices = vec![String::from("Default")];
    if let Ok(output) = tokio::process::Command::new("say")
        .arg("-a")
        .arg("?")
        .output()
        .await
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let name = line.trim();
            if !name.is_empty() && !devices.iter().any(|device| device == name) {
                devices.push(name.to_string());
            }
        }
    }
    devices
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::alerts::{AlertEngine, AlertKind, ProbeSnapshot, Snapshot};
    use crate::clock::AppInstant;
    use crate::model::{ProbeId, Unit};
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Semaphore;

    #[derive(Default)]
    struct Log(StdMutex<Vec<String>>);

    impl Log {
        fn push(&self, line: String) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(line);
        }
        fn entries(&self) -> Vec<String> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    struct AbortGuard {
        log: Arc<Log>,
        text: String,
        completed: Arc<AtomicBool>,
    }

    impl Drop for AbortGuard {
        fn drop(&mut self) {
            if !self.completed.load(Ordering::SeqCst) {
                self.log.push(format!("abort:{}", self.text));
            }
        }
    }

    struct FakeSpeaker {
        log: Arc<Log>,
        gate: Arc<Semaphore>,
    }

    impl Speaker for FakeSpeaker {
        fn start(&mut self, text: &str, _settings: &SpeechSettings) -> SpeechFuture {
            let text = text.to_string();
            self.log.push(format!("start:{text}"));
            let log = Arc::clone(&self.log);
            let gate = Arc::clone(&self.gate);
            Box::pin(async move {
                let completed = Arc::new(AtomicBool::new(false));
                let guard = AbortGuard {
                    log: Arc::clone(&log),
                    text: text.clone(),
                    completed: Arc::clone(&completed),
                };
                let _permit = gate.acquire().await.map_err(|err| err.to_string())?;
                completed.store(true, Ordering::SeqCst);
                drop(guard);
                log.push(format!("done:{text}"));
                Ok(())
            })
        }
    }

    async fn spawn_worker(
        settings: SpeechSettings,
        force_off: bool,
        shared: Arc<Mutex<ActiveAlerts>>,
    ) -> (
        mpsc::Sender<SpeechMsg>,
        Arc<Log>,
        tokio::task::JoinHandle<()>,
    ) {
        let log = Arc::new(Log::default());
        let mut speaker = Box::new(FakeSpeaker {
            log: Arc::clone(&log),
            gate: Arc::new(Semaphore::new(0)),
        });
        let (tx, mut rx) = mpsc::channel(16);
        let clock = AppClock::new(1.0);
        let mut settings = settings;
        let handle = tokio::spawn(async move {
            let clock_ref: &dyn Clock = &clock;
            run_worker(
                speaker.as_mut(),
                clock_ref,
                &shared,
                &mut settings,
                force_off,
                &mut rx,
            )
            .await;
        });
        (tx, log, handle)
    }

    async fn wait_for(log: &Arc<Log>, needle: &str) {
        for _ in 0..200 {
            if log.entries().iter().any(|line| line.contains(needle)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {needle:?} in {:?}", log.entries());
    }

    #[test]
    fn queue_dedupes_and_prioritises() {
        let mut queue = SpeechQueue::new();
        let status = SpeechItem {
            key: SpeechKey::Status,
            priority: Priority::Normal,
        };
        let preview = SpeechItem {
            key: SpeechKey::Preview,
            priority: Priority::Urgent,
        };
        queue.push(status);
        queue.push(status);
        queue.push(preview);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.take_next().unwrap().key, SpeechKey::Preview);
        assert_eq!(queue.take_next().unwrap().key, SpeechKey::Status);
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn urgent_only_skips_normal() {
        let settings = SpeechSettings {
            urgent_only: true,
            ..SpeechSettings::default()
        };
        let shared = Arc::new(Mutex::new(ActiveAlerts::default()));
        let (tx, log, _handle) = spawn_worker(settings, false, shared).await;
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Status,
            priority: Priority::Normal,
        }))
        .await
        .unwrap();
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Preview,
            priority: Priority::Urgent,
        }))
        .await
        .unwrap();
        wait_for(&log, "start:This is the bbq-watch voice preview.").await;
        assert!(
            !log.entries()
                .iter()
                .any(|line| line.contains("start:No probes")),
            "normal item must not be spoken: {:?}",
            log.entries()
        );
    }

    #[tokio::test]
    async fn cleared_alert_is_dropped() {
        let shared = Arc::new(Mutex::new(ActiveAlerts::default()));
        let (tx, log, _handle) = spawn_worker(SpeechSettings::default(), false, shared).await;
        let key = (Some(ProbeId::new(1).unwrap()), AlertKind::TargetReached);
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Alert(key),
            priority: Priority::Urgent,
        }))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            log.entries().is_empty(),
            "stale item was spoken: {:?}",
            log.entries()
        );
    }

    #[tokio::test]
    async fn active_alert_is_spoken() {
        let id = ProbeId::new(1).unwrap();
        let snapshot = Snapshot {
            probes: vec![ProbeSnapshot {
                id,
                label: String::from("P1"),
                temp_c: Some(crate::model::fahrenheit_to_celsius(212.0)),
                target_c: Some(crate::model::fahrenheit_to_celsius(203.0)),
                alert_high_c: None,
                alert_low_c: None,
                pre_alert_c: None,
                is_pit: false,
                range_c: None,
                stalled: false,
            }],
            battery: None,
            last_reading_at: Some(AppInstant::ZERO),
            serve_late: false,
            alert_on_stall: true,
        };
        let mut engine = AlertEngine::new(90, 2, 30);
        let _ = engine.evaluate(&snapshot, AppInstant::ZERO);
        let mut active = ActiveAlerts::default();
        active.sync(
            &engine,
            &snapshot,
            Unit::Fahrenheit,
            None,
            None,
            AppInstant::ZERO,
        );
        let shared = Arc::new(Mutex::new(active));
        let (tx, log, _handle) = spawn_worker(SpeechSettings::default(), false, shared).await;
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Alert((Some(id), AlertKind::TargetReached)),
            priority: Priority::Urgent,
        }))
        .await
        .unwrap();
        wait_for(&log, "start:P1 is at 212 degrees").await;
    }

    #[tokio::test]
    async fn urgent_preempts_normal() {
        let shared = Arc::new(Mutex::new(ActiveAlerts::default()));
        let (tx, log, _handle) = spawn_worker(SpeechSettings::default(), false, shared).await;
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Status,
            priority: Priority::Normal,
        }))
        .await
        .unwrap();
        wait_for(&log, "start:No probes are reading.").await;
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Preview,
            priority: Priority::Urgent,
        }))
        .await
        .unwrap();
        wait_for(&log, "start:This is the bbq-watch voice preview.").await;
        assert!(
            log.entries()
                .iter()
                .any(|line| line == "abort:No probes are reading."),
            "normal item must be aborted: {:?}",
            log.entries()
        );
    }

    #[tokio::test]
    async fn urgent_does_not_preempt_urgent() {
        let shared = Arc::new(Mutex::new(ActiveAlerts::default()));
        let (tx, log, _handle) = spawn_worker(SpeechSettings::default(), false, shared).await;
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Preview,
            priority: Priority::Urgent,
        }))
        .await
        .unwrap();
        wait_for(&log, "start:This is the bbq-watch voice preview.").await;
        tx.send(SpeechMsg::Item(SpeechItem {
            key: SpeechKey::Test,
            priority: Priority::Urgent,
        }))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !log.entries().iter().any(|line| line.starts_with("abort:")),
            "urgent item must not pre-empt another urgent item: {:?}",
            log.entries()
        );
    }
}
