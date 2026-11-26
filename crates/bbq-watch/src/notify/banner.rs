//! macOS notification banner via `osascript`.

/// Show a notification banner titled `bbq-watch`.
pub async fn show(message: &str) -> Result<(), super::NotifyError> {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{escaped}\" with title \"bbq-watch\"");
    super::run_command("osascript", &["-e", script.as_str()]).await
}
