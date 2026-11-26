//! Keep-awake child process (`caffeinate`), tied to our own pid.

use std::process::Stdio;

use tokio::process::Child;

/// Owns the `caffeinate -dimsu -w <pid>` child. `set` only runs when the
/// desired state changes, so calling it on every settings change is cheap;
/// `poll` notices a child that has exited without respawning it every tick.
#[derive(Debug, Default)]
pub struct Caffeinate {
    child: Option<Child>,
    desired: bool,
    /// A spawn that failed (e.g. the binary is missing); do not retry until
    /// the setting changes.
    failed: bool,
}

impl Caffeinate {
    /// No child running yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the desired state and reconcile immediately. A no-op when the
    /// state is unchanged, so a held setting never respawns the child.
    pub fn set(&mut self, enabled: bool) {
        if enabled == self.desired {
            return;
        }
        self.desired = enabled;
        self.failed = false;
        if enabled {
            self.ensure();
        } else {
            self.kill();
        }
    }

    /// Detect a child that has exited and restart it if still wanted. Cheap
    /// enough to call every tick; never spawns twice.
    pub fn poll(&mut self) {
        if let Some(child) = self.child.as_mut()
            && !matches!(child.try_wait(), Ok(None))
        {
            self.child = None;
        }
        self.ensure();
    }

    /// Whether the keep-awake child is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.child.is_some()
    }

    /// Stop the child (used on shutdown).
    pub fn stop(&mut self) {
        self.desired = false;
        self.kill();
    }

    /// Spawn the child if it is wanted, not running and not failed.
    fn ensure(&mut self) {
        if !self.desired || self.child.is_some() || self.failed {
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
            Err(err) => {
                self.failed = true;
                tracing::warn!("caffeinate could not start: {err}");
            }
        }
    }

    /// Kill the child if it is running.
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Best-effort kill; the child also has kill_on_drop set.
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn poll_never_respawns_a_live_child() {
        let mut caffeinate = Caffeinate::new();
        caffeinate.set(true);
        let id = caffeinate
            .child
            .as_ref()
            .and_then(tokio::process::Child::id);
        for _ in 0..50 {
            caffeinate.poll();
        }
        assert_eq!(
            caffeinate
                .child
                .as_ref()
                .and_then(tokio::process::Child::id),
            id
        );
        caffeinate.stop();
    }

    #[test]
    fn a_failed_spawn_is_not_retried_every_poll() {
        // Force the failed state without a real spawn.
        let mut caffeinate = Caffeinate {
            desired: true,
            failed: true,
            child: None,
        };
        for _ in 0..50 {
            caffeinate.poll();
        }
        assert!(!caffeinate.is_active());
    }
}
