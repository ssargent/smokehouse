//! `bbq-watch` binary entry point: parse the CLI, start file logging and hand
//! control to [`bbq_watch::app::run`].

use anyhow::Context as _;
use clap::Parser as _;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = bbq_watch::cli::Cli::parse();
    let _log_guard = init_tracing()?;
    bbq_watch::app::run(cli).await
}

/// Send diagnostics to a daily-rolling file; never to the terminal.
fn init_tracing() -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    let dir = bbq_watch::config::data_dir()?;
    std::fs::create_dir_all(&dir).context("creating data directory")?;
    let appender = tracing_appender::rolling::daily(&dir, "bbq-watch.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Ok(guard)
}
