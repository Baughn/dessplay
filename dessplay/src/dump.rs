use anyhow::Result;

use dessplay_core::crdt::CrdtState;

use crate::storage::ClientStorage;

pub fn dump_database(storage: &ClientStorage) -> Result<()> {
    println!("=== DessPlay Client Database Dump ===\n");

    dump_config(storage)?;
    dump_media_roots(storage)?;
    dump_crdt_state(storage)?;
    dump_watch_history(storage)?;
    dump_file_mappings(storage)?;
    dump_tofu_certs(storage)?;

    Ok(())
}

fn dump_config(storage: &ClientStorage) -> Result<()> {
    println!("--- Config ---");
    match storage.get_config()? {
        Some(config) => {
            println!("  Username: {}", config.username);
            println!("  Server:   {}", config.server);
            println!("  Player:   {}", config.player);
            println!(
                "  Password: {}",
                if config.password.is_some() {
                    "(set)"
                } else {
                    "(none)"
                }
            );
        }
        None => println!("  (no config)"),
    }
    println!();
    Ok(())
}

fn dump_media_roots(storage: &ClientStorage) -> Result<()> {
    println!("--- Media Roots ---");
    let roots = storage.get_media_roots()?;
    if roots.is_empty() {
        println!("  (none)");
    } else {
        for (i, root) in roots.iter().enumerate() {
            println!("  {i}: {}", root.display());
        }
    }
    println!();
    Ok(())
}

fn dump_crdt_state(storage: &ClientStorage) -> Result<()> {
    println!("--- CRDT State ---");
    match storage.load_latest_snapshot()? {
        Some((epoch, snapshot)) => {
            let mut state = CrdtState::new();
            state.load_snapshot(epoch, snapshot);
            let ops = storage.load_ops(epoch)?;
            println!("  Epoch: {epoch}");
            println!("  Ops in current epoch: {}", ops.len());
            for op in &ops {
                state.apply_op(op);
            }
            println!("{state:#?}");
        }
        None => {
            println!("  (no CRDT snapshot)");
            // Check if there are ops without a snapshot (epoch 0)
            let ops = storage.load_ops(0)?;
            if !ops.is_empty() {
                println!("  Ops at epoch 0: {}", ops.len());
                let mut state = CrdtState::new();
                for op in &ops {
                    state.apply_op(op);
                }
                println!("{state:#?}");
            }
        }
    }
    println!();
    Ok(())
}

fn dump_watch_history(storage: &ClientStorage) -> Result<()> {
    println!("--- Watch History ---");
    let watched = storage.watched_files()?;
    if watched.is_empty() {
        println!("  (none)");
    } else {
        for (file_id, timestamp) in &watched {
            println!("  {file_id}  last_watched_at={timestamp}");
        }
    }
    println!();
    Ok(())
}

fn dump_file_mappings(storage: &ClientStorage) -> Result<()> {
    println!("--- File Mappings ---");
    let mappings = storage.get_all_file_mappings()?;
    if mappings.is_empty() {
        println!("  (none)");
    } else {
        for (file_id, path) in &mappings {
            println!("  {file_id} -> {}", path.display());
        }
    }
    println!();
    Ok(())
}

fn dump_tofu_certs(storage: &ClientStorage) -> Result<()> {
    println!("--- TOFU Certificates ---");
    let certs = storage.get_all_tofu_certs()?;
    if certs.is_empty() {
        println!("  (none)");
    } else {
        for cert in &certs {
            let fp_hex: String = cert.fingerprint.iter().map(|b| format!("{b:02x}")).collect();
            println!(
                "  {}  fingerprint={}  first_seen_at={}",
                cert.server_address, fp_hex, cert.first_seen_at
            );
        }
    }
    println!();
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::storage::{ClientStorage, Config};
    use dessplay_core::protocol::{CrdtOp, LwwValue, PlaylistAction};
    use dessplay_core::types::{FileId, UserId, UserState};

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    #[test]
    fn dump_empty_database() {
        let db = ClientStorage::open_in_memory().unwrap();
        dump_database(&db).unwrap();
    }

    #[test]
    fn dump_populated_database() {
        let db = ClientStorage::open_in_memory().unwrap();

        // Config
        db.save_config(&Config {
            username: "alice".into(),
            server: "dessplay.brage.info".into(),
            player: "mpv".into(),
            password: Some("secret".into()),
        })
        .unwrap();

        // Media roots
        db.set_media_roots(&["/anime".into(), "/shows".into()])
            .unwrap();

        // CRDT state
        let mut state = CrdtState::new();
        state.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::UserState(UserId("alice".into()), UserState::Ready),
        });
        state.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 200,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        });
        state.apply_op(&CrdtOp::ChatAppend {
            user_id: UserId("bob".into()),
            seq: 0,
            timestamp: 300,
            text: "hello world".into(),
        });
        db.save_snapshot(1, &state.snapshot()).unwrap();
        db.append_op(
            1,
            &CrdtOp::LwwWrite {
                timestamp: 400,
                value: LwwValue::UserState(UserId("bob".into()), UserState::Paused),
            },
        )
        .unwrap();

        // Watch history
        db.mark_watched(&fid(1), 1000).unwrap();
        db.mark_watched(&fid(2), 2000).unwrap();

        // File mappings
        let tmpdir = tempfile::tempdir().unwrap();
        let tmpfile = tmpdir.path().join("01.mkv");
        std::fs::write(&tmpfile, b"test data").unwrap();
        db.set_file_mapping(&fid(3), &tmpfile).unwrap();

        // TOFU cert
        db.store_cert("dessplay.brage.info", &[0xDE, 0xAD, 0xBE, 0xEF])
            .unwrap();

        dump_database(&db).unwrap();
    }
}
