use std::io::{self, Stdout};
use std::path::PathBuf;

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tracing_appender::non_blocking::WorkerGuard;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// RAII guard that restores the terminal on drop.
pub struct TerminalGuard {
    pub terminal: Tui,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

/// Set up the terminal for TUI rendering.
pub fn setup_terminal() -> Result<TerminalGuard> {
    // Install panic hook that restores terminal first
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    enable_raw_mode().context("failed to enable raw mode")?;
    crossterm::execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend).context("failed to create terminal")?;

    Ok(TerminalGuard { terminal })
}

/// Restore the terminal to its original state.
fn restore_terminal() -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
        .context("failed to leave alternate screen")?;
    Ok(())
}

/// Set up tracing to write to a log file instead of stderr (raw mode corrupts stderr).
pub fn setup_file_logging() -> Result<WorkerGuard> {
    let log_dir = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine data directory"))?
        .join("dessplay");
    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log dir {}", log_dir.display()))?;

    let log_path = log_dir.join("dessplay.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open log file {}", log_path.display()))?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    Ok(guard)
}

/// Path where the TUI log file lives.
pub fn log_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("dessplay").join("dessplay.log"))
}
