//! Push alerts to <https://ntfy.sh>.

use std::time::Duration;

use reqwest::Client;

/// POST a message to `https://ntfy.sh/<topic>`.
pub async fn push(client: &Client, topic: &str, message: &str, urgent: bool) -> Result<(), String> {
    let url = format!("https://ntfy.sh/{topic}");
    let priority = if urgent { "urgent" } else { "default" };
    let response = client
        .post(url)
        .header("Title", "bbq-watch")
        .header("Priority", priority)
        .timeout(Duration::from_secs(10))
        .body(message.to_string())
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("ntfy returned {}", response.status()))
    }
}
