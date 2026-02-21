use anyhow::Result;

use dessplay_core::crdt::CrdtState;

use crate::storage::ServerStorage;

pub fn dump_database(storage: &ServerStorage) -> Result<()> {
    println!("=== DessPlay Rendezvous Server Database Dump ===\n");

    dump_crdt_state(storage)?;
    dump_anidb_queue(storage)?;

    Ok(())
}

fn dump_crdt_state(storage: &ServerStorage) -> Result<()> {
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

fn dump_anidb_queue(storage: &ServerStorage) -> Result<()> {
    println!("--- AniDB Queue ---");
    let entries = storage.get_all_anidb_queue()?;
    if entries.is_empty() {
        println!("  (empty)");
    } else {
        println!("  {} entries:", entries.len());
        for entry in &entries {
            let checked = match entry.last_checked_at {
                Some(ts) => ts.to_string(),
                None => "never".to_string(),
            };
            println!(
                "  {}  has_data={}  first_seen={}  last_checked={}  next_check={}  retries={}",
                entry.file_id,
                entry.has_data,
                entry.first_seen_at,
                checked,
                entry.next_check_at,
                entry.retry_count,
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
    use crate::storage::ServerStorage;
    use dessplay_core::protocol::{CrdtOp, LwwValue, PlaylistAction};
    use dessplay_core::types::{FileId, UserId, UserState};

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    #[test]
    fn dump_empty_database() {
        let db = ServerStorage::open_in_memory().unwrap();
        dump_database(&db).unwrap();
    }

    #[test]
    fn dump_populated_database() {
        let db = ServerStorage::open_in_memory().unwrap();

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
        db.save_snapshot(1, &state.snapshot()).unwrap();
        db.append_op(
            1,
            &CrdtOp::ChatAppend {
                user_id: UserId("bob".into()),
                seq: 0,
                timestamp: 300,
                text: "hello".into(),
            },
        )
        .unwrap();

        // AniDB queue
        db.enqueue_anidb_lookup(&fid(1), 1_000_000).unwrap();
        db.enqueue_anidb_lookup(&fid(2), 2_000_000).unwrap();
        db.record_success(&fid(1), 1_500_000).unwrap();
        db.record_failure(&fid(2), 2_500_000).unwrap();

        dump_database(&db).unwrap();
    }
}
