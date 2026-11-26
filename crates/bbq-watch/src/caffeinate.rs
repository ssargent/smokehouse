//! Keep-awake child process (`caffeinate`), tied to our own pid.

use std::process::Stdio;

use tokio::process::Child;

/// Owns the `caffeinate -dimsu -w <pid>` child, started and stopped on demand.
#[derive(Debug, Default)]
pub struct Caffeinate {
    child: Option<Child>,
}

impl Caffeinate {
    /// No child running yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start or stop the child to match `enabled`.
    pub async fn set(&mut self, enabled: bool) {
        if enabled {
            self.start().await;
        } else {
            self.stop().await;
        }
    }

    /// Whether the keep-awake child is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.child.is_some()
    }

    async fn start(&mut self) {
        if self.child.is_some() {
            return;
        }
        let pid = std::process::id().to_string();
        let result = tokio::process::Command::new("caffeinate")
            .arg("-dimsu")
            .arg("-w")
            .arg(pid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn();
        match result {
            Ok(child) => self.child = Some(child),
            Err(err) => tracing::warn!("caffeinate could not start: {err}"),
        }
    }

    /// Kill the child if it is running.
    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Best-effort kill; the child also has kill_on_drop set.
            let _ = child.kill().await;
        }
    }
}
