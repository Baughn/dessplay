//! DessPlay client binary — a thin shell over the library (see
//! architecture.md, Composition Root). Phase 5: headless client and
//! seeder mode; the TUI arrives in Phase 6.

use clap::Parser;
use dessplay::run::{
    HeadlessArgs, load_dotenv, run_dump, run_headless, run_import, run_interactive,
};

/// A synchronized video player for watch parties.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Run headless as a seeder: no TUI, no player, never gates
    /// playback. Configured purely by flags/env; persists nothing.
    #[arg(long)]
    seeder: bool,

    /// Rendezvous server, `host[:port]`. Overrides the stored setting.
    #[arg(long)]
    server: Option<String>,

    /// Username. Overrides the stored setting.
    #[arg(long)]
    username: Option<String>,

    /// Room password. Overrides DESSPLAY_PASSWORD and the stored
    /// setting.
    #[arg(long)]
    password: Option<String>,

    /// Hex SHA-256 fingerprint of the server certificate to pin
    /// (mainly for seeders, which persist nothing).
    #[arg(long)]
    fingerprint: Option<String>,

    /// Settings/state database path. Overrides the default; honored in
    /// every mode (run a second instance with its own --db and
    /// --cache-dir).
    #[arg(long)]
    db: Option<std::path::PathBuf>,

    /// Outstanding chunk requests per source for downloads (default 48).
    #[arg(long)]
    pipeline_depth: Option<u32>,

    /// Media library to search/serve (repeatable). Overrides the stored
    /// media roots for this run (not persisted); for a seeder, the only
    /// way to set them. The download cache is always served too.
    #[arg(long = "media-root")]
    media_root: Vec<std::path::PathBuf>,

    /// Download cache directory (defaults to the standard cache).
    /// Overrides the default in every mode.
    #[arg(long)]
    cache_dir: Option<std::path::PathBuf>,

    /// Run headless (no TUI) even as an interactive user.
    #[arg(long)]
    headless: bool,

    /// Attach to an mpv you launched yourself at this IPC socket instead
    /// of spawning one — a dev/headless aid for working without a desktop.
    /// Launch mpv with, e.g.,
    /// `mpv --idle=yes --keep-open=yes --vo=tct --input-ipc-server=<socket>`
    /// (the `--idle --keep-open` matter; `--vo=tct` shows video in the
    /// terminal). dessplay leaves that mpv running on exit.
    #[arg(long, value_name = "SOCKET")]
    attach_mpv: Option<std::path::PathBuf>,

    /// Print stored settings and CRDT state as JSON on stdout, then exit
    /// (logs go to stderr). Use `--section` to trim the output.
    #[arg(long)]
    dump: bool,

    /// Restrict `--dump` to these sections (repeatable). Valid names:
    /// settings, media_roots, playlist, watched, now_playing,
    /// seek_authority, playback_intent, series_preference,
    /// manual_override, file_availability, anidb_metadata,
    /// series_relations, file_catalog, list_entries, list_next_ep,
    /// lookup_requests, chat, playback_position, acknowledged_absent.
    /// Omit to dump everything.
    #[arg(long = "section", value_name = "SECTION", requires = "dump")]
    section: Vec<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// One-shot import of the tracking spreadsheet (CSV exports, one
    /// file per sheet) into The List. Re-imports update existing
    /// entries by name instead of duplicating them.
    ImportList {
        /// Exported sheet CSVs.
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
        /// Watcher initials mapping, e.g. `B=Baughn,N=Nero`.
        #[arg(long, default_value = "B=Baughn,N=Nero,Q=Quickshot,D=Dagger,K=Kim")]
        watchers: String,
        /// Parse and report only; submit nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    load_dotenv();
    let cli = Cli::parse();
    let interactive = cli.command.is_none() && !cli.seeder && !cli.headless && !cli.dump;
    // The TUI owns the screen: route logs to a file there. Without
    // this, supervisory failures (a crashed thread, a wedged shutdown)
    // are completely invisible.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let log_dir = if interactive {
        dessplay::logging::log_dir()
    } else {
        None
    };
    if interactive {
        // Split logs into one file per biblical day and drop anything
        // older than a week (and the legacy unitary dessplay.log) before
        // opening today's file.
        if let Some(dir) = &log_dir {
            dessplay::logging::trim_old_logs(dir, dessplay::logging::today_biblical(), 7);
        }
        match log_dir.clone() {
            Some(dir) => tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(dessplay::logging::BiblicalDailyWriter::new(dir))
                .with_ansi(false)
                .init(),
            None => tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::io::sink)
                .init(),
        }
        tracing::info!("dessplay {} starting", env!("CARGO_PKG_VERSION"));
    } else if cli.dump {
        // `--dump` writes JSON to stdout; keep logs off it so the output
        // is machine-parseable.
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    // Panics land in the log before the default hook prints them (the
    // terminal adapter's own hook restores the screen first).
    {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!("panic: {info}");
            default_hook(info);
        }));
    }

    let args = HeadlessArgs {
        seeder: cli.seeder,
        server: cli.server,
        username: cli.username,
        password: cli.password,
        fingerprint: cli.fingerprint,
        db_path: cli.db,
        pipeline_depth: cli.pipeline_depth,
        media_roots: cli.media_root,
        cache_dir: cli.cache_dir,
        attach_mpv: cli.attach_mpv,
    };
    if cli.dump {
        if let Err(message) = run_dump(&args, &cli.section) {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let result = match cli.command {
        None if interactive => runtime.block_on(run_interactive(args)),
        None => runtime.block_on(run_headless(args)),
        Some(Command::ImportList {
            files,
            watchers,
            dry_run,
        }) => runtime.block_on(run_import(args, files, watchers, dry_run)),
    };
    if let Err(message) = result {
        eprintln!("error: {message}");
        if let Some(dir) = &log_dir {
            eprintln!(
                "(log: {})",
                dessplay::logging::current_log_path(dir).display()
            );
        }
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use clap::CommandFactory;

    /// The `--pipeline-depth` help must state the real production default
    /// (48, set in `run::download_config`), not the stale 16. Regression
    /// for the help/code drift in the 2026-06-26 review.
    #[test]
    fn pipeline_depth_help_states_the_production_default() {
        // Normalize line-wrapping so the assertion doesn't depend on the
        // terminal width clap renders at.
        let help: String = Cli::command()
            .render_long_help()
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            help.contains("default 48"),
            "--pipeline-depth help should state the production default 48:\n{help}"
        );
        assert!(
            !help.contains("default 16"),
            "--pipeline-depth help still states the stale default 16:\n{help}"
        );
    }
}
