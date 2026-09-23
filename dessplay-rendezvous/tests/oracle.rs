//! The oracle (design.md, Oracle) end to end: a Seeder-role client
//! pumped the way `run_headless` pumps it, with a scripted model,
//! against the real server over the simulated transport. Paused time
//! throughout.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::*;
use dessplay::actors::sync::{Mutation, SyncEvent};
use dessplay::client::ClientEvent;
use dessplay::oracle::{Oracle, OracleError, OracleModel, OracleRequest};
use dessplay_core::StateView;
use dessplay_core::types::UserId;

/// Answers with the question echoed and counts calls.
struct Echo(Arc<AtomicUsize>);

impl OracleModel for Echo {
    fn answer(&self, req: &OracleRequest) -> Result<String, OracleError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(format!("answer to: {}", req.question))
    }
}

/// Spawn the oracle client and its pump; returns the task so a test
/// can kill it (a process restart).
fn spawn_oracle(
    harness: &Harness,
    nonce: u128,
    calls: Arc<AtomicUsize>,
) -> tokio::task::JoinHandle<()> {
    let mut handle = harness.seeder("oracle", nonce);
    let clock = sim_clock(0);
    let (mut oracle, mut results) = Oracle::new(UserId::new("oracle"), Arc::new(Echo(calls)));
    tokio::spawn(async move {
        loop {
            tokio::select! {
                event = handle.events.recv() => {
                    let Some(event) = event else { break };
                    if let ClientEvent::Sync(SyncEvent::StateChanged) = event {
                        let adopted = *handle.state_adopted.borrow();
                        oracle.on_state_changed(&handle.sync, adopted, clock()).await;
                    }
                }
                outcome = results.recv() => {
                    let Some(outcome) = outcome else { break };
                    oracle.on_outcome(outcome, &handle.sync).await;
                }
            }
        }
    })
}

fn oracle_lines(view: &StateView) -> Vec<String> {
    view.chat
        .iter()
        .filter(|m| m.sender == UserId::new("oracle"))
        .map(|m| m.text.clone())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn the_oracle_answers_new_questions_once_and_never_history() {
    init_test_logging();
    let harness = Harness::new(0x0AC1E);
    let kim = harness.client("kim", 1);
    // A question from before the oracle existed is history.
    mutate(
        &kim,
        Mutation::Chat {
            text: "oracle: old news?".into(),
        },
    )
    .await;
    eventually_views(&[&kim], Duration::from_secs(10), |v| v[0].chat.len() == 1).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let first = spawn_oracle(&harness, 2, Arc::clone(&calls));
    eventually(&[&kim], Duration::from_secs(30), |s| {
        s[0].peer("oracle").is_some()
    })
    .await;
    // Let the oracle adopt state and baseline the history.
    tokio::time::sleep(Duration::from_secs(5)).await;

    mutate(
        &kim,
        Mutation::Chat {
            text: "oracle: are there foxes on Okinawa?".into(),
        },
    )
    .await;
    eventually_views(&[&kim], Duration::from_secs(30), |v| {
        oracle_lines(&v[0]) == ["answer to: are there foxes on Okinawa?"]
    })
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "history was not answered");

    // A reconnect re-syncs the whole state: nothing is re-answered.
    harness.isolate("oracle");
    tokio::time::sleep(Duration::from_secs(20)).await;
    harness.heal("oracle");
    tokio::time::sleep(Duration::from_secs(30)).await;
    // A restart is a fresh stateless client: its baseline covers
    // everything already said.
    first.abort();
    let second = spawn_oracle(&harness, 3, Arc::clone(&calls));
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "re-answered after reconnect or restart"
    );
    assert_eq!(oracle_lines(&view_of(&kim).await).len(), 1);

    // And the restarted oracle still answers new questions.
    mutate(
        &kim,
        Mutation::Chat {
            text: "Oracle: and on Hokkaido?".into(),
        },
    )
    .await;
    eventually_views(&[&kim], Duration::from_secs(30), |v| {
        oracle_lines(&v[0]).len() == 2
    })
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    second.abort();
}
