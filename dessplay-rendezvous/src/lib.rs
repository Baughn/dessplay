//! DessPlay rendezvous server library. The binary in `main.rs` is a
//! thin shell so tests (including cross-crate connection tests) can
//! construct and run the server in-process.

#![deny(missing_docs)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::todo)]
#![deny(clippy::dbg_macro)]

pub mod anidb;
pub mod server;
pub mod storage;

/// Load `./.env` (KEY=VALUE lines, optionally `export `-prefixed; `#`
/// comments) into the environment, without overriding variables that
/// are already set. Call before spawning any threads.
pub fn load_dotenv() {
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, value)) = line.split_once('=') {
            let (key, value) = (key.trim(), value.trim().trim_matches('"'));
            if std::env::var_os(key).is_none() {
                // Single-threaded startup: set_var is safe here.
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
}
