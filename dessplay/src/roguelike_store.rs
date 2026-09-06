//! Transactional local saves and a durable chat-report outbox for the roguelike.
//!
//! Commands always begin from the committed save. The returned run is safe to
//! display only after the save commits, so a failed disk write cannot advance
//! the visible game. A completed run and its report enter storage atomically.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::roguelike::{Action, Run};
use crate::storage::{Result, Storage, StorageError};

const SAVE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u32,
    run: Run,
}

/// A player command that must cross the durable-save boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Resume the current run, creating the first run if necessary.
    Open,
    /// Apply one action to the current run.
    Act(Action),
    /// Start again after a completed run; a living run is never replaced.
    NewRun,
}

/// A completed expedition awaiting publication to shared chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Stable local history-row identity, used to acknowledge delivery.
    pub id: i64,
    /// Original completion time in Unix milliseconds; retain this on retries.
    pub timestamp: i64,
    /// Player-facing epitaph, including its stable expedition number.
    pub summary: String,
}

fn corrupt(message: impl std::fmt::Display) -> StorageError {
    StorageError::Corrupt(format!("roguelike save: {message}"))
}

fn decode(save: &str) -> Result<Run> {
    // Check the envelope version before decoding engine fields. Future formats
    // are never mistaken for a missing save, even if their shape has changed.
    let value: serde_json::Value = serde_json::from_str(save).map_err(corrupt)?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(u64::from(SAVE_VERSION)) {
        return Err(corrupt("unsupported or missing format version"));
    }
    let envelope: Envelope = serde_json::from_value(value).map_err(corrupt)?;
    envelope.run.validate().map_err(corrupt)?;
    Ok(envelope.run)
}

/// Apply a command, returning only a committed run.
///
/// `seed` is consumed only when creating an expedition; `now_ms` is used only
/// for its completion report. Both come from the caller so replay tests never
/// depend on clocks or ambient randomness. Corrupt saves are returned as errors
/// and retained intact, including when requesting a new run.
pub fn handle(
    storage: &Storage,
    user: &str,
    command: Command,
    seed: u64,
    now_ms: i64,
) -> Result<Run> {
    storage.roguelike_transaction(|transaction| {
        let saved: Option<(i64, String)> = transaction
            .query_row(
                "SELECT generation, save FROM roguelike_runs WHERE username = ?1",
                [user],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (mut generation, mut run, mut changed) = match saved {
            Some((generation, save)) => (generation, decode(&save)?, false),
            None => (1, Run::new(seed), true),
        };
        match command {
            Command::Open => {}
            Command::Act(action) => changed |= run.act(action),
            Command::NewRun if run.is_finished() => {
                generation = generation
                    .checked_add(1)
                    .ok_or_else(|| corrupt("expedition counter exhausted"))?;
                run = Run::new(seed);
                changed = true;
            }
            Command::NewRun => {}
        }
        if changed {
            run.validate().map_err(corrupt)?;
            let save = serde_json::to_string(&Envelope {
                version: SAVE_VERSION,
                run: run.clone(),
            })
            .map_err(corrupt)?;
            transaction.execute(
                "INSERT INTO roguelike_runs (username, generation, save) VALUES (?1, ?2, ?3)
                 ON CONFLICT (username) DO UPDATE SET
                    generation = excluded.generation, save = excluded.save",
                params![user, generation, save],
            )?;
            if run.is_finished() {
                transaction.execute(
                    "INSERT INTO roguelike_history
                        (username, generation, summary, ended_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (username, generation) DO NOTHING",
                    params![
                        user,
                        generation,
                        format!("{} [expedition #{generation}]", run.summary()),
                        now_ms
                    ],
                )?;
            }
        }
        Ok(run)
    })
}

impl Storage {
    /// Read pending reports in completion order, including earlier expeditions.
    ///
    /// Reading is nondestructive: retry with the same summary and timestamp
    /// until the sync actor confirms durable publication, then acknowledge it.
    pub fn pending_roguelike_reports(&self, user: &str) -> Result<Vec<Report>> {
        self.roguelike_transaction(|transaction| {
            let mut statement = transaction.prepare(
                "SELECT id, ended_at, summary FROM roguelike_history
                 WHERE username = ?1 AND reported = 0 ORDER BY id",
            )?;
            let reports = statement.query_map([user], |row| {
                Ok(Report {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    summary: row.get(2)?,
                })
            })?;
            Ok(reports.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Mark a report delivered while retaining its permanent local history.
    pub fn ack_roguelike_report(&self, id: i64) -> Result<()> {
        self.roguelike_transaction(|transaction| {
            transaction.execute(
                "UPDATE roguelike_history SET reported = 1 WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn open(storage: &Storage, user: &str, seed: u64) -> Run {
        handle(storage, user, Command::Open, seed, 100).unwrap()
    }

    fn raw_save(storage: &Storage, user: &str) -> String {
        storage
            .roguelike_transaction(|transaction| {
                Ok(transaction.query_row(
                    "SELECT save FROM roguelike_runs WHERE username = ?1",
                    [user],
                    |row| row.get(0),
                )?)
            })
            .unwrap()
    }

    /// Stop through the honest action interface immediately before death, so
    /// failure injection exercises the actual living -> finished transaction.
    fn before_death(storage: &Storage, user: &str) -> Run {
        let mut run = open(storage, user, 17);
        for _ in 0..10_000 {
            let mut next = run.clone();
            next.act(Action::Wait);
            if next.is_finished() {
                // Build the starvation prefix through real simulation actions,
                // seed its validated save once, then exercise the fatal action
                // and outbox through the real transactional interface.
                run.validate().unwrap();
                let save = serde_json::to_string(&Envelope {
                    version: SAVE_VERSION,
                    run: run.clone(),
                })
                .unwrap();
                storage
                    .roguelike_transaction(|tx| {
                        tx.execute(
                            "UPDATE roguelike_runs SET save=?1 WHERE username=?2",
                            params![save, user],
                        )?;
                        Ok(())
                    })
                    .unwrap();
                return run;
            }
            run = next;
        }
        panic!("waiting without food should eventually finish the expedition");
    }

    #[test]
    fn reopen_resumes_exact_state_and_rng_per_user() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client.db");
        let alice;
        let bob;
        {
            let storage = Storage::open(&path).unwrap();
            open(&storage, "Alice", 17);
            bob = open(&storage, "Bob", 29);
            alice = handle(&storage, "Alice", Command::Act(Action::Wait), 99, 101).unwrap();
        }
        let storage = Storage::open(&path).unwrap();
        assert_eq!(open(&storage, "Alice", 999), alice);
        assert_eq!(open(&storage, "Bob", 999), bob);
        let mut expected = alice;
        expected.act(Action::Wait);
        assert_eq!(
            handle(&storage, "Alice", Command::Act(Action::Wait), 99, 102).unwrap(),
            expected
        );
        assert!(
            storage
                .pending_roguelike_reports("Alice")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn new_run_never_discards_a_living_expedition() {
        let storage = Storage::open_in_memory().unwrap();
        let first = open(&storage, "Alice", 17);
        assert_eq!(
            handle(&storage, "Alice", Command::NewRun, 29, 100).unwrap(),
            first
        );
        assert_eq!(open(&storage, "Alice", 29), first);
    }

    #[test]
    fn invalid_and_future_saves_are_preserved_for_every_command() {
        let storage = Storage::open_in_memory().unwrap();
        open(&storage, "Alice", 17);
        let mut invalid_run: serde_json::Value =
            serde_json::from_str(&raw_save(&storage, "Alice")).unwrap();
        invalid_run["run"]["depth"] = serde_json::json!(999);
        let invalid_run = invalid_run.to_string();
        for bad_save in [
            "not JSON",
            r#"{"version":2,"run":{}}"#,
            r#"{"version":1,"run":{}}"#,
            &invalid_run,
        ] {
            storage
                .roguelike_transaction(|transaction| {
                    transaction.execute(
                        "UPDATE roguelike_runs SET save = ?1 WHERE username = 'Alice'",
                        [bad_save],
                    )?;
                    Ok(())
                })
                .unwrap();
            for command in [Command::Open, Command::Act(Action::Wait), Command::NewRun] {
                assert!(handle(&storage, "Alice", command, 29, 100).is_err());
                assert_eq!(raw_save(&storage, "Alice"), bad_save);
            }
        }
    }

    #[test]
    fn failed_save_does_not_advance_committed_turn() {
        let storage = Storage::open_in_memory().unwrap();
        let first = open(&storage, "Alice", 17);
        storage
            .roguelike_transaction(|transaction| {
                transaction.execute_batch(
                    "CREATE TRIGGER fail_save BEFORE UPDATE ON roguelike_runs
                     BEGIN SELECT RAISE(ABORT, 'injected disk failure'); END;",
                )?;
                Ok(())
            })
            .unwrap();
        assert!(handle(&storage, "Alice", Command::Act(Action::Wait), 0, 100).is_err());
        assert_eq!(open(&storage, "Alice", 17), first);
    }

    #[test]
    fn repeated_seed_and_timestamp_still_have_distinct_report_identities() {
        let storage = Storage::open_in_memory().unwrap();
        for _ in 0..2 {
            before_death(&storage, "Alice");
            handle(&storage, "Alice", Command::Act(Action::Wait), 17, 300).unwrap();
            handle(&storage, "Alice", Command::NewRun, 17, 300).unwrap();
        }
        let reports = storage.pending_roguelike_reports("Alice").unwrap();
        assert_eq!(reports.len(), 2);
        assert_ne!(reports[0].id, reports[1].id);
        assert_eq!(reports[0].timestamp, reports[1].timestamp);
        assert_ne!(reports[0].summary, reports[1].summary);
        assert!(reports[0].summary.ends_with("[expedition #1]"));
        assert!(reports[1].summary.ends_with("[expedition #2]"));
    }

    #[test]
    fn finish_and_report_are_atomic_and_retryable_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("client.db");
        let report;
        {
            let storage = Storage::open(&path).unwrap();
            let living = before_death(&storage, "Alice");
            storage
                .roguelike_transaction(|transaction| {
                    transaction.execute_batch(
                        "CREATE TRIGGER fail_report BEFORE INSERT ON roguelike_history
                         BEGIN SELECT RAISE(ABORT, 'injected disk failure'); END;",
                    )?;
                    Ok(())
                })
                .unwrap();
            assert!(handle(&storage, "Alice", Command::Act(Action::Wait), 0, 300).is_err());
            assert_eq!(open(&storage, "Alice", 0), living);
            assert!(
                storage
                    .pending_roguelike_reports("Alice")
                    .unwrap()
                    .is_empty()
            );
            storage
                .roguelike_transaction(|transaction| {
                    transaction.execute_batch("DROP TRIGGER fail_report")?;
                    Ok(())
                })
                .unwrap();
            let dead = handle(&storage, "Alice", Command::Act(Action::Wait), 0, 300).unwrap();
            assert!(dead.is_finished());
            let reports = storage.pending_roguelike_reports("Alice").unwrap();
            assert_eq!(reports.len(), 1);
            report = reports[0].clone();
            assert_eq!(report.timestamp, 300);
            assert!(report.summary.contains(&dead.summary()));
            assert!(storage.pending_roguelike_reports("Bob").unwrap().is_empty());
            // Reopening and further action do not emit a second report.
            assert_eq!(open(&storage, "Alice", 0), dead);
            handle(&storage, "Alice", Command::Act(Action::Wait), 0, 400).unwrap();
            assert_eq!(storage.pending_roguelike_reports("Alice").unwrap(), reports);
            // A fresh run does not discard the previous run's pending report.
            assert!(
                !handle(&storage, "Alice", Command::NewRun, 29, 400)
                    .unwrap()
                    .is_finished()
            );
        }
        let storage = Storage::open(&path).unwrap();
        assert_eq!(
            storage.pending_roguelike_reports("Alice").unwrap(),
            std::slice::from_ref(&report)
        );
        storage.ack_roguelike_report(report.id).unwrap();
        storage.ack_roguelike_report(report.id).unwrap();
        drop(storage);
        let storage = Storage::open(&path).unwrap();
        assert!(
            storage
                .pending_roguelike_reports("Alice")
                .unwrap()
                .is_empty()
        );
        storage
            .roguelike_transaction(|transaction| {
                let count: i64 = transaction.query_row(
                    "SELECT count(*) FROM roguelike_history WHERE username = 'Alice'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 1);
                Ok(())
            })
            .unwrap();
    }
}
