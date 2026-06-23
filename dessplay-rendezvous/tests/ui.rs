//! Phase 6: the harness with UI handles — keystrokes into one client's
//! real `Ui` dispatcher propagate through the real server and render
//! in another client's buffer (testing-strategy.md's scenario shape).

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::*;
use dessplay::actors::sync::SyncCommand;
use dessplay::config::Settings;
use dessplay::ui::app::{Ui, UiSnapshot};
use dessplay::ui::msg::UserAction;
use dessplay_core::types::UserId;
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;
use tuirealm::testing::buffer_to_string;

fn key(code: Key) -> Event<NoUserEvent> {
    Event::Keyboard(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
    })
}

/// A client plus its UI: input goes through the real dispatcher, and
/// actions are applied to the real sync actor.
struct UiClient {
    handle: dessplay::client::ClientHandle,
    ui: Ui,
}

impl UiClient {
    fn new(harness: &Harness, name: &str, nonce: u128) -> Self {
        let handle = harness.client(name, nonce);
        let settings = Settings {
            username: Some(name.into()),
            password: Some(PASSWORD.into()),
            ..Settings::default()
        };
        Self {
            handle,
            ui: Ui::new(UserId::new(name), settings, vec!["/media".into()]),
        }
    }

    /// Pull a fresh snapshot into the UI.
    async fn sync_ui(&mut self) {
        let snapshot = UiSnapshot {
            view: std::sync::Arc::new(view_of(&self.handle).await),
            peers: self.handle.peers.borrow().clone(),
            recency: BTreeMap::new(),
            cache_hashes: Default::default(),
        };
        self.ui.apply_snapshot(snapshot);
    }

    /// Feed keys through the dispatcher, applying resulting mutations
    /// to the sync actor (the production bridge, minus the threads).
    async fn input(&mut self, events: impl IntoIterator<Item = Event<NoUserEvent>>) {
        for event in events {
            for action in self.ui.handle(event) {
                match action {
                    UserAction::Mutate(mutation) => {
                        self.handle
                            .sync
                            .send(SyncCommand::Mutate(Box::new(mutation)))
                            .await
                            .unwrap();
                    }
                    other => panic!("unexpected action in this test: {other:?}"),
                }
            }
        }
    }

    fn render(&mut self) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let completed = terminal.draw(|frame| self.ui.draw(frame)).unwrap();
        buffer_to_string(completed.buffer)
    }
}

#[tokio::test(start_paused = true)]
async fn chat_typed_in_one_ui_renders_in_the_other() {
    let harness = Harness::new(0x5EED);
    let mut kim = UiClient::new(&harness, "kim", 1);
    let mut baughn = UiClient::new(&harness, "baughn", 2);

    // Kim types a message through her real chat pane.
    kim.input("konnichiwa".chars().map(|c| key(Key::Char(c))))
        .await;
    kim.input([key(Key::Enter)]).await;

    // It reaches baughn's state...
    eventually(
        &[&kim.handle, &baughn.handle],
        Duration::from_secs(30),
        |snaps| snaps.iter().all(|s| s.view.chat.len() == 1),
    )
    .await;

    // ...and his screen, with sender attribution and the peer list.
    baughn.sync_ui().await;
    let screen = baughn.render();
    assert!(screen.contains("kim: konnichiwa"), "{screen}");
    assert!(screen.contains("kim [ready]"), "{screen}");
    assert!(screen.contains("baughn [ready]"), "{screen}");
}

#[tokio::test(start_paused = true)]
async fn away_marked_in_one_ui_shows_on_the_other() {
    let harness = Harness::new(0x5EED);
    let mut kim = UiClient::new(&harness, "kim", 1);
    let mut baughn = UiClient::new(&harness, "baughn", 2);

    // Both UIs need the peer list before the users pane can act on it.
    eventually(
        &[&kim.handle, &baughn.handle],
        Duration::from_secs(30),
        |snaps| snaps.iter().all(|s| s.peers.len() == 2),
    )
    .await;
    kim.sync_ui().await;

    // Kim tabs to the users pane and marks baughn away (first row —
    // peers sort alphabetically).
    kim.input([key(Key::Tab), key(Key::Tab), key(Key::Char('a'))])
        .await;

    eventually(&[&baughn.handle], Duration::from_secs(30), |snaps| {
        !snaps[0].view.manual_override.is_empty()
    })
    .await;
    baughn.sync_ui().await;
    let screen = baughn.render();
    assert!(screen.contains("baughn [away, set by kim]"), "{screen}");
}
