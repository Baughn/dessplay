//! Durable dungeon turns and epitaphs through the production SessionLoop.

mod common;

use std::time::Duration;

use common::{Harness, LoopRig, init_test_logging, loop_rig};
use dessplay::roguelike::{Action, Point, Run, RunView, Tile};
use dessplay::roguelike_store::{Command, handle};
use dessplay::storage::Storage;
use dessplay::ui::msg::UserAction;
use dessplay::ui::shell::UiInput;
use rusqlite::OptionalExtension;

async fn command(rig: &LoopRig, command: Command) -> RunView {
    rig.actions
        .send(UserAction::Roguelike(command))
        .await
        .expect("session loop alive");
    receive_run(rig).await
}

async fn receive_run(rig: &LoopRig) -> RunView {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            while let Ok(input) = rig.ui_rx.try_recv() {
                if let UiInput::Roguelike(result) = input {
                    return *result.expect("turn committed");
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("game response reached UI")
}

async fn epitaph(rig: &LoopRig, summary: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let view = rig.view().await;
            let count = view
                .chat
                .iter()
                .filter(|line| line.sender.0 == "kim" && line.text == summary)
                .count();
            assert!(
                count <= 1,
                "duplicate epitaph in remote chat: {:?}",
                view.chat
            );
            if count == 1 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("epitaph reached another player");
}

async fn acknowledged(storage: &Storage) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !storage.pending_roguelike_reports("kim").unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("published report acknowledged locally");
}

fn before_death(storage: &Storage, path: &std::path::Path) -> Run {
    let mut run = handle(storage, "kim", Command::Open, 17, 100).unwrap();
    for _ in 0..10_000 {
        let mut next = run.clone();
        next.act(Action::Wait);
        if next.is_finished() {
            // The long starvation prefix uses real engine actions. Persist its
            // validated state once, then exercise the fatal action, transaction,
            // session reply, and report delivery through production interfaces.
            run.validate().unwrap();
            let conn = rusqlite::Connection::open(path).unwrap();
            let saved: String = conn
                .query_row(
                    "SELECT save FROM roguelike_runs WHERE username='kim'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let mut envelope: serde_json::Value = serde_json::from_str(&saved).unwrap();
            envelope["run"] = serde_json::to_value(&run).unwrap();
            conn.execute(
                "UPDATE roguelike_runs SET save=?1 WHERE username='kim'",
                [envelope.to_string()],
            )
            .unwrap();
            return run;
        }
        run = next;
    }
    panic!("waiting without food should eventually end a run");
}

#[tokio::test(flavor = "multi_thread")]
async fn committed_game_reply_survives_a_full_ui_queue() {
    init_test_logging();
    let harness = Harness::new(90504);
    let dir = tempfile::tempdir().unwrap();
    let rig = loop_rig(&harness, "kim", 1, dir.path());
    // LoopRig has 64 UI slots. Keep its receiver idle while enough ordered
    // notices pass through the session loop to fill every slot before Open.
    for index in 0..128 {
        rig.actions
            .send(UserAction::Notice(format!("busy UI {index}")))
            .await
            .unwrap();
    }
    rig.actions
        .send(UserAction::Roguelike(Command::Open))
        .await
        .unwrap();

    // Observe the real commit without consuming a UI message or accidentally
    // creating the save ourselves through the Open command's fallback.
    let connection = rusqlite::Connection::open_with_flags(
        dir.path().join("kim.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let save = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let save: Option<String> = connection
                .query_row(
                    "SELECT save FROM roguelike_runs WHERE username = 'kim'",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .unwrap();
            if let Some(save) = save {
                return save;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("game committed despite the full UI queue");
    let saved: serde_json::Value = serde_json::from_str(&save).unwrap();
    let committed: Run = serde_json::from_value(saved["run"].clone()).unwrap();
    assert_eq!(receive_run(&rig).await, committed.view());

    // Receiving the retained reply frees the UI's waiting-for-save state, so
    // the next action must be accepted and return another committed snapshot.
    let next = command(&rig, Command::Act(Action::Wait)).await;
    assert_eq!(next.turns, committed.turns + 1);
    rig.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn modal_turn_reply_is_persisted_and_resumes_after_client_restart() {
    init_test_logging();
    let harness = Harness::new(90501);
    let dir = tempfile::tempdir().unwrap();
    let rig = loop_rig(&harness, "kim", 1, dir.path());
    let initial = command(&rig, Command::Open).await;
    let (dx, dy) = [(0, 1), (1, 0), (0, -1), (-1, 0)]
        .into_iter()
        .find(|(dx, dy)| {
            initial.tile(Point {
                x: initial.position.x + dx,
                y: initial.position.y + dy,
            }) != Tile::Wall
        })
        .expect("entrance has a walkable neighbor");
    let moved = command(&rig, Command::Act(Action::Move(dx, dy))).await;
    assert_eq!(moved.turns, initial.turns + 1);
    let storage = Storage::open(&dir.path().join("kim.db")).unwrap();
    assert_eq!(
        handle(&storage, "kim", Command::Open, 0, 0).unwrap().view(),
        moved
    );
    drop(storage);
    rig.quit().await;
    let resumed = loop_rig(&harness, "kim", 2, dir.path());
    assert_eq!(command(&resumed, Command::Open).await, moved);
    resumed.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dying_in_the_modal_publishes_one_summary_to_another_player() {
    init_test_logging();
    let harness = Harness::new(90502);
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let storage = Storage::open(&dir_a.path().join("kim.db")).unwrap();
    let living = before_death(&storage, &dir_a.path().join("kim.db"));
    let rig_a = loop_rig(&harness, "kim", 1, dir_a.path());
    let rig_b = loop_rig(&harness, "nero", 1, dir_b.path());
    assert_eq!(command(&rig_a, Command::Open).await, living.view());
    let dead = command(&rig_a, Command::Act(Action::Wait)).await;
    assert!(dead.is_finished());
    let summary = format!("{} [expedition #1]", dead.summary());
    epitaph(&rig_b, &summary).await;
    assert_eq!(command(&rig_a, Command::Open).await, dead);
    assert_eq!(command(&rig_a, Command::Act(Action::Wait)).await, dead);
    epitaph(&rig_b, &summary).await;
    acknowledged(&storage).await;
    rig_a.quit().await;
    let resumed = loop_rig(&harness, "kim", 2, dir_a.path());
    assert_eq!(command(&resumed, Command::Open).await, dead);
    epitaph(&rig_b, &summary).await;
    resumed.quit().await;
    rig_b.quit().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn startup_publishes_a_saved_death_even_without_opening_the_modal() {
    init_test_logging();
    let harness = Harness::new(90503);
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let storage = Storage::open(&dir_a.path().join("kim.db")).unwrap();
    before_death(&storage, &dir_a.path().join("kim.db"));
    // Model a process stopping after its fatal turn commits, before the
    // session loop has submitted the durable outbox entry to the sync actor.
    let dead = handle(
        &storage,
        "kim",
        Command::Act(Action::Wait),
        0,
        1_700_000_000_000,
    )
    .unwrap();
    assert!(dead.is_finished());
    let pending = storage.pending_roguelike_reports("kim").unwrap();
    assert_eq!(pending.len(), 1);
    let rig_b = loop_rig(&harness, "nero", 1, dir_b.path());
    let rig_a = loop_rig(&harness, "kim", 1, dir_a.path());
    epitaph(&rig_b, &pending[0].summary).await;
    acknowledged(&storage).await;
    rig_a.quit().await;
    rig_b.quit().await;
}
