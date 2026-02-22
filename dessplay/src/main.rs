use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use dessplay::storage;

#[tokio::main]
async fn main() -> Result<()> {
    // Install the ring crypto provider for rustls (must happen before any TLS use)
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls CryptoProvider"))?;

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: dessplay [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --server <ADDR>  Server address (default from config)");
        println!("  --db <PATH>      Database path (default: ~/.local/share/dessplay/dessplay.db)");
        println!("  --dump           Read and pretty-print the local database to stdout");
        println!("  --help           Show this help message");
        return Ok(());
    }

    // Open storage
    let db_path = get_arg(&args, "--db")
        .map(PathBuf::from)
        .map_or_else(|| storage::default_db_path(), Ok)?;
    let storage = Arc::new(Mutex::new(storage::ClientStorage::open(&db_path)?));

    if args.iter().any(|a| a == "--dump") {
        let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        dessplay::dump::dump_database(&s)?;
        return Ok(());
    }

    // Run the TUI (handles settings screen, connection, and main loop)
    dessplay::tui::runner::run(storage, &args).await
}

/// Extract the value after a `--flag` from the arg list.
fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
