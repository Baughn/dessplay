use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};

use dessplay_core::protocol::{CrdtOp, CrdtSnapshot};
use dessplay_core::types::FileId;

// ---------------------------------------------------------------------------
// Config type
// ---------------------------------------------------------------------------

/// Local user configuration, persisted in SQLite.
pub struct Config {
    pub username: String,
    pub server: String,
    pub player: String,
    pub password: Option<String>,
}

/// TOFU certificate entry, as stored in SQLite.
#[derive(Debug)]
pub struct TofuCert {
    pub server_address: String,
    pub fingerprint: Vec<u8>,
    pub first_seen_at: u64,
}

// ---------------------------------------------------------------------------
// Client storage
// ---------------------------------------------------------------------------

pub struct ClientStorage {
    conn: Connection,
}

impl ClientStorage {
    /// Open (or create) the database at the given path and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    // -----------------------------------------------------------------------
    // Migrations
    // -----------------------------------------------------------------------

    fn migrate(&self) -> Result<()> {
        let version = self.schema_version()?;
        if version < 1 {
            self.migrate_v1()?;
        }
        Ok(())
    }

    fn schema_version(&self) -> Result<u32> {
        let table_exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='metadata'",
            [],
            |row| row.get(0),
        )?;
        if !table_exists {
            return Ok(0);
        }
        let version: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match version {
            Some(v) => Ok(v.parse::<u32>().context("invalid schema_version in metadata")?),
            None => Ok(0),
        }
    }

    fn migrate_v1(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS config (
                id       INTEGER PRIMARY KEY CHECK(id = 1),
                username TEXT NOT NULL,
                server   TEXT NOT NULL DEFAULT 'dessplay.brage.info',
                player   TEXT NOT NULL DEFAULT 'mpv',
                password TEXT
            );

            CREATE TABLE IF NOT EXISTS media_roots (
                sort_order INTEGER PRIMARY KEY,
                path       TEXT NOT NULL UNIQUE
            );

            CREATE TABLE IF NOT EXISTS crdt_snapshots (
                epoch      INTEGER PRIMARY KEY,
                data       BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS crdt_ops (
                id    INTEGER PRIMARY KEY AUTOINCREMENT,
                epoch INTEGER NOT NULL,
                data  BLOB NOT NULL
            );

            CREATE TABLE IF NOT EXISTS watch_history (
                file_hash       BLOB PRIMARY KEY,
                last_watched_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS file_mappings (
                file_hash  BLOB PRIMARY KEY,
                local_path TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tofu_certs (
                server_address TEXT PRIMARY KEY,
                fingerprint    BLOB NOT NULL,
                first_seen_at  INTEGER NOT NULL
            );",
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', '1')",
            [],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Config
    // -----------------------------------------------------------------------

    pub fn get_config(&self) -> Result<Option<Config>> {
        self.conn
            .query_row(
                "SELECT username, server, player, password FROM config WHERE id = 1",
                [],
                |row| {
                    Ok(Config {
                        username: row.get(0)?,
                        server: row.get(1)?,
                        player: row.get(2)?,
                        password: row.get(3)?,
                    })
                },
            )
            .optional()
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO config (id, username, server, player, password) \
             VALUES (1, ?1, ?2, ?3, ?4)",
            params![config.username, config.server, config.player, config.password],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Media roots
    // -----------------------------------------------------------------------

    pub fn get_media_roots(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM media_roots ORDER BY sort_order")?;
        let roots = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                Ok(PathBuf::from(path))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(roots)
    }

    pub fn set_media_roots(&self, roots: &[PathBuf]) -> Result<()> {
        self.conn.execute("DELETE FROM media_roots", [])?;
        let mut stmt = self
            .conn
            .prepare("INSERT INTO media_roots (sort_order, path) VALUES (?1, ?2)")?;
        for (i, root) in roots.iter().enumerate() {
            let path_str = root
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("media root path is not valid UTF-8: {root:?}"))?;
            stmt.execute(params![i as i64, path_str])?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // CRDT persistence
    // -----------------------------------------------------------------------

    pub fn save_snapshot(&self, epoch: u64, snapshot: &CrdtSnapshot) -> Result<()> {
        let data = postcard::to_allocvec(snapshot).context("failed to serialize CRDT snapshot")?;
        self.conn.execute(
            "INSERT OR REPLACE INTO crdt_snapshots (epoch, data, created_at) VALUES (?1, ?2, ?3)",
            params![epoch as i64, data, now_millis()],
        )?;
        Ok(())
    }

    pub fn load_latest_snapshot(&self) -> Result<Option<(u64, CrdtSnapshot)>> {
        let row: Option<(i64, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT epoch, data FROM crdt_snapshots ORDER BY epoch DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match row {
            Some((epoch, data)) => {
                let snapshot: CrdtSnapshot =
                    postcard::from_bytes(&data).context("failed to deserialize CRDT snapshot")?;
                Ok(Some((epoch as u64, snapshot)))
            }
            None => Ok(None),
        }
    }

    pub fn append_op(&self, epoch: u64, op: &CrdtOp) -> Result<()> {
        let data = postcard::to_allocvec(op).context("failed to serialize CRDT op")?;
        self.conn.execute(
            "INSERT INTO crdt_ops (epoch, data) VALUES (?1, ?2)",
            params![epoch as i64, data],
        )?;
        Ok(())
    }

    pub fn load_ops(&self, epoch: u64) -> Result<Vec<CrdtOp>> {
        let mut stmt = self
            .conn
            .prepare("SELECT data FROM crdt_ops WHERE epoch = ?1 ORDER BY id")?;
        let blobs = stmt
            .query_map(params![epoch as i64], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        blobs
            .iter()
            .map(|data| postcard::from_bytes(data).context("failed to deserialize CRDT op"))
            .collect()
    }

    pub fn delete_before_epoch(&self, epoch: u64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM crdt_snapshots WHERE epoch < ?1",
            params![epoch as i64],
        )?;
        self.conn.execute(
            "DELETE FROM crdt_ops WHERE epoch < ?1",
            params![epoch as i64],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Watch history
    // -----------------------------------------------------------------------

    pub fn mark_watched(&self, file_id: &FileId, timestamp: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO watch_history (file_hash, last_watched_at) VALUES (?1, ?2)",
            params![file_id.0.as_slice(), timestamp as i64],
        )?;
        Ok(())
    }

    pub fn is_watched(&self, file_id: &FileId) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM watch_history WHERE file_hash = ?1",
            params![file_id.0.as_slice()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn watched_files(&self) -> Result<Vec<(FileId, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_hash, last_watched_at FROM watch_history ORDER BY last_watched_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let hash: Vec<u8> = row.get(0)?;
                let ts: i64 = row.get(1)?;
                Ok((hash, ts))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(hash, ts)| {
                ensure!(
                    hash.len() == 16,
                    "corrupt file_hash in watch_history: expected 16 bytes, got {}",
                    hash.len()
                );
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&hash);
                Ok((FileId(arr), ts as u64))
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // File mappings
    // -----------------------------------------------------------------------

    pub fn set_file_mapping(&self, file_id: &FileId, path: &Path) -> Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("file mapping path is not valid UTF-8: {path:?}"))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO file_mappings (file_hash, local_path) VALUES (?1, ?2)",
            params![file_id.0.as_slice(), path_str],
        )?;
        Ok(())
    }

    pub fn get_file_mapping(&self, file_id: &FileId) -> Result<Option<PathBuf>> {
        self.conn
            .query_row(
                "SELECT local_path FROM file_mappings WHERE file_hash = ?1",
                params![file_id.0.as_slice()],
                |row| {
                    let path: String = row.get(0)?;
                    Ok(PathBuf::from(path))
                },
            )
            .optional()
    }

    // -----------------------------------------------------------------------
    // TOFU certificates
    // -----------------------------------------------------------------------

    pub fn store_cert(&self, server: &str, fingerprint: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tofu_certs (server_address, fingerprint, first_seen_at) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(server_address) DO UPDATE SET fingerprint = excluded.fingerprint",
            params![server, fingerprint, now_millis()],
        )?;
        Ok(())
    }

    pub fn delete_cert(&self, server: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM tofu_certs WHERE server_address = ?1",
            params![server],
        )?;
        Ok(())
    }

    pub fn get_cert(&self, server: &str) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT fingerprint FROM tofu_certs WHERE server_address = ?1",
                params![server],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn get_all_tofu_certs(&self) -> Result<Vec<TofuCert>> {
        let mut stmt = self.conn.prepare(
            "SELECT server_address, fingerprint, first_seen_at FROM tofu_certs \
             ORDER BY server_address",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(server_address, fingerprint, first_seen_at)| TofuCert {
                server_address,
                fingerprint,
                first_seen_at: first_seen_at as u64,
            })
            .collect())
    }

    pub fn get_all_file_mappings(&self) -> Result<Vec<(FileId, PathBuf)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_hash, local_path FROM file_mappings ORDER BY local_path")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(hash, path)| {
                ensure!(
                    hash.len() == 16,
                    "corrupt file_hash in file_mappings: expected 16 bytes, got {}",
                    hash.len()
                );
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&hash);
                Ok((FileId(arr), PathBuf::from(path)))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Default DB path
// ---------------------------------------------------------------------------

pub fn default_db_path() -> Result<PathBuf> {
    let data_dir =
        dirs::data_dir().ok_or_else(|| anyhow::anyhow!("could not determine XDG data directory"))?;
    Ok(data_dir.join("dessplay").join("dessplay.db"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Extension trait for converting `QueryReturnedNoRows` into `Ok(None)`.
trait OptionalRow<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalRow<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use dessplay_core::protocol::{CrdtOp, LwwValue, PlaylistAction};
    use dessplay_core::types::{FileId, UserState};

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    fn uid(name: &str) -> dessplay_core::types::UserId {
        dessplay_core::types::UserId(name.to_string())
    }

    #[test]
    fn migration_creates_tables() {
        let db = ClientStorage::open_in_memory().unwrap();
        // Verify all tables exist by querying sqlite_master
        let tables: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(tables.contains(&"metadata".to_string()));
        assert!(tables.contains(&"config".to_string()));
        assert!(tables.contains(&"media_roots".to_string()));
        assert!(tables.contains(&"crdt_snapshots".to_string()));
        assert!(tables.contains(&"crdt_ops".to_string()));
        assert!(tables.contains(&"watch_history".to_string()));
        assert!(tables.contains(&"file_mappings".to_string()));
        assert!(tables.contains(&"tofu_certs".to_string()));
    }

    #[test]
    fn migration_is_idempotent() {
        let db = ClientStorage::open_in_memory().unwrap();
        // Running migrate again should not fail
        db.migrate().unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    #[test]
    fn config_round_trip() {
        let db = ClientStorage::open_in_memory().unwrap();
        assert!(db.get_config().unwrap().is_none());

        let config = Config {
            username: "alice".into(),
            server: "example.com".into(),
            player: "mpv".into(),
            password: Some("secret".into()),
        };
        db.save_config(&config).unwrap();

        let loaded = db.get_config().unwrap().unwrap();
        assert_eq!(loaded.username, "alice");
        assert_eq!(loaded.server, "example.com");
        assert_eq!(loaded.player, "mpv");
        assert_eq!(loaded.password, Some("secret".into()));
    }

    #[test]
    fn config_update() {
        let db = ClientStorage::open_in_memory().unwrap();
        db.save_config(&Config {
            username: "alice".into(),
            server: "a.com".into(),
            player: "mpv".into(),
            password: None,
        })
        .unwrap();

        db.save_config(&Config {
            username: "bob".into(),
            server: "b.com".into(),
            player: "vlc".into(),
            password: Some("pw".into()),
        })
        .unwrap();

        let loaded = db.get_config().unwrap().unwrap();
        assert_eq!(loaded.username, "bob");
        assert_eq!(loaded.server, "b.com");
    }

    #[test]
    fn media_roots_ordering() {
        let db = ClientStorage::open_in_memory().unwrap();
        let roots = vec![
            PathBuf::from("/anime"),
            PathBuf::from("/shows"),
            PathBuf::from("/downloads"),
        ];
        db.set_media_roots(&roots).unwrap();
        assert_eq!(db.get_media_roots().unwrap(), roots);
    }

    #[test]
    fn media_roots_replace() {
        let db = ClientStorage::open_in_memory().unwrap();
        db.set_media_roots(&[PathBuf::from("/old")]).unwrap();
        let new_roots = vec![PathBuf::from("/new1"), PathBuf::from("/new2")];
        db.set_media_roots(&new_roots).unwrap();
        assert_eq!(db.get_media_roots().unwrap(), new_roots);
    }

    #[test]
    fn snapshot_round_trip() {
        let db = ClientStorage::open_in_memory().unwrap();

        // Build a CrdtState with some data
        let mut state = dessplay_core::crdt::CrdtState::new();
        state.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::UserState(uid("alice"), UserState::Ready),
        });
        state.apply_op(&CrdtOp::PlaylistOp {
            timestamp: 200,
            action: PlaylistAction::Add {
                file_id: fid(1),
                after: None,
            },
        });
        state.apply_op(&CrdtOp::ChatAppend {
            user_id: uid("bob"),
            seq: 0,
            timestamp: 300,
            text: "hello".into(),
        });

        let snapshot = state.snapshot();
        db.save_snapshot(1, &snapshot).unwrap();

        let (epoch, loaded) = db.load_latest_snapshot().unwrap().unwrap();
        assert_eq!(epoch, 1);
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn snapshot_latest_wins() {
        let db = ClientStorage::open_in_memory().unwrap();
        let mut state = dessplay_core::crdt::CrdtState::new();

        db.save_snapshot(1, &state.snapshot()).unwrap();

        state.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::UserState(uid("alice"), UserState::Paused),
        });
        db.save_snapshot(2, &state.snapshot()).unwrap();

        let (epoch, _) = db.load_latest_snapshot().unwrap().unwrap();
        assert_eq!(epoch, 2);
    }

    #[test]
    fn op_log_round_trip() {
        let db = ClientStorage::open_in_memory().unwrap();

        let ops = vec![
            CrdtOp::LwwWrite {
                timestamp: 10,
                value: LwwValue::UserState(uid("alice"), UserState::Ready),
            },
            CrdtOp::PlaylistOp {
                timestamp: 20,
                action: PlaylistAction::Add {
                    file_id: fid(1),
                    after: None,
                },
            },
            CrdtOp::ChatAppend {
                user_id: uid("bob"),
                seq: 0,
                timestamp: 30,
                text: "hi".into(),
            },
        ];

        for op in &ops {
            db.append_op(1, op).unwrap();
        }

        let loaded = db.load_ops(1).unwrap();
        assert_eq!(loaded.len(), 3);

        // Replay into a fresh CrdtState and verify
        let mut original = dessplay_core::crdt::CrdtState::new();
        for op in &ops {
            original.apply_op(op);
        }
        let mut replayed = dessplay_core::crdt::CrdtState::new();
        for op in &loaded {
            replayed.apply_op(op);
        }
        assert_eq!(original.snapshot(), replayed.snapshot());
    }

    #[test]
    fn epoch_cleanup() {
        let db = ClientStorage::open_in_memory().unwrap();
        let state = dessplay_core::crdt::CrdtState::new();

        db.save_snapshot(1, &state.snapshot()).unwrap();
        db.save_snapshot(2, &state.snapshot()).unwrap();
        db.save_snapshot(3, &state.snapshot()).unwrap();
        db.append_op(
            1,
            &CrdtOp::LwwWrite {
                timestamp: 1,
                value: LwwValue::UserState(uid("a"), UserState::Ready),
            },
        )
        .unwrap();
        db.append_op(
            3,
            &CrdtOp::LwwWrite {
                timestamp: 2,
                value: LwwValue::UserState(uid("b"), UserState::Ready),
            },
        )
        .unwrap();

        db.delete_before_epoch(3).unwrap();

        // Epoch 3 snapshot still exists
        let (epoch, _) = db.load_latest_snapshot().unwrap().unwrap();
        assert_eq!(epoch, 3);

        // Epoch 1 ops are gone, epoch 3 ops remain
        assert!(db.load_ops(1).unwrap().is_empty());
        assert_eq!(db.load_ops(3).unwrap().len(), 1);
    }

    #[test]
    fn watch_history() {
        let db = ClientStorage::open_in_memory().unwrap();
        let f1 = fid(1);
        let f2 = fid(2);

        assert!(!db.is_watched(&f1).unwrap());

        db.mark_watched(&f1, 1000).unwrap();
        assert!(db.is_watched(&f1).unwrap());
        assert!(!db.is_watched(&f2).unwrap());

        db.mark_watched(&f2, 2000).unwrap();
        let watched = db.watched_files().unwrap();
        assert_eq!(watched.len(), 2);
        // Ordered by last_watched_at DESC
        assert_eq!(watched[0].0, f2);
        assert_eq!(watched[1].0, f1);
    }

    #[test]
    fn watch_history_update_timestamp() {
        let db = ClientStorage::open_in_memory().unwrap();
        let f1 = fid(1);

        db.mark_watched(&f1, 1000).unwrap();
        db.mark_watched(&f1, 5000).unwrap();

        let watched = db.watched_files().unwrap();
        assert_eq!(watched.len(), 1);
        assert_eq!(watched[0].1, 5000);
    }

    #[test]
    fn file_mappings() {
        let db = ClientStorage::open_in_memory().unwrap();
        let f1 = fid(1);
        let path = PathBuf::from("/anime/Frieren/01.mkv");

        assert!(db.get_file_mapping(&f1).unwrap().is_none());

        db.set_file_mapping(&f1, &path).unwrap();
        assert_eq!(db.get_file_mapping(&f1).unwrap(), Some(path.clone()));

        // Overwrite
        let new_path = PathBuf::from("/anime/Frieren/01v2.mkv");
        db.set_file_mapping(&f1, &new_path).unwrap();
        assert_eq!(db.get_file_mapping(&f1).unwrap(), Some(new_path));
    }

    #[test]
    fn tofu_certs() {
        let db = ClientStorage::open_in_memory().unwrap();
        let server = "dessplay.brage.info";
        let fp = vec![0xDE, 0xAD, 0xBE, 0xEF];

        assert!(db.get_cert(server).unwrap().is_none());

        db.store_cert(server, &fp).unwrap();
        assert_eq!(db.get_cert(server).unwrap(), Some(fp));

        // Update fingerprint
        let new_fp = vec![0xCA, 0xFE];
        db.store_cert(server, &new_fp).unwrap();
        assert_eq!(db.get_cert(server).unwrap(), Some(new_fp));
    }

    #[test]
    fn no_snapshot_returns_none() {
        let db = ClientStorage::open_in_memory().unwrap();
        assert!(db.load_latest_snapshot().unwrap().is_none());
    }

    #[test]
    fn no_ops_returns_empty() {
        let db = ClientStorage::open_in_memory().unwrap();
        assert!(db.load_ops(1).unwrap().is_empty());
    }
}
