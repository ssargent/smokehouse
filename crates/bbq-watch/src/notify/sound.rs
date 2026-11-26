//! Alert sound via macOS `afplay`.

/// Play a sound from `/System/Library/Sounds`.
pub async fn play(file: &str) -> Result<(), String> {
    let path = format!("/System/Library/Sounds/{file}");
    super::run_command("afplay", &[path.as_str()]).await
}
