mod dump;
#[allow(dead_code)]
mod storage;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: dessplay [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --dump    Read and pretty-print the local database to stdout");
        println!("  --help    Show this help message");
        return Ok(());
    }

    if args.iter().any(|a| a == "--dump") {
        let db_path = storage::default_db_path()?;
        let storage = storage::ClientStorage::open(&db_path)?;
        dump::dump_database(&storage)?;
        return Ok(());
    }

    Ok(())
}
