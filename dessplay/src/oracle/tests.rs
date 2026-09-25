#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::BTreeSet;

use dessplay_core::playlist::{NewPlaylistEntry, PlaylistEntry};
use dessplay_core::types::{
    ActorId, AniDbMetadata, Ed2kHash, MetadataSource, PlaybackPosition, SharedTimestamp,
    encode_action,
};
use proptest::prelude::*;

use super::*;

fn msg(sender: &str, at: u64, text: &str) -> ChatMessage {
    ChatMessage {
        timestamp: SharedTimestamp::from_millis(at),
        sender: UserId::new(sender),
        text: text.into(),
    }
}

// ---- Trigger parsing -----------------------------------------------------

#[test]
fn triggers_are_prefix_case_insensitive_and_need_a_question() {
    assert_eq!(
        parse_trigger("oracle: are there foxes on Okinawa?"),
        Some("are there foxes on Okinawa?")
    );
    assert_eq!(parse_trigger("  Oracle:why?"), Some("why?"));
    assert_eq!(parse_trigger("ORACLE:  x  "), Some("x"));
    assert_eq!(parse_trigger("oracle:"), None);
    assert_eq!(parse_trigger("oracle:   "), None);
    assert_eq!(parse_trigger("oracle, what?"), None);
    assert_eq!(parse_trigger("hey oracle: what?"), None);
    assert_eq!(parse_trigger("orac"), None);
    // A multibyte first character must not panic the prefix slice.
    assert_eq!(parse_trigger("ørakel: hva?"), None);
}

// ---- ChatWatch -----------------------------------------------------------

#[test]
fn history_at_adoption_is_never_answered() {
    let mut watch = ChatWatch::new(UserId::new("oracle"));
    let history = vec![msg("alice", 1_000, "oracle: old question")];
    assert!(watch.observe(&history, false, 1_000).is_empty());
    assert!(watch.observe(&history, true, 1_000).is_empty());
    let mut chat = history.clone();
    chat.push(msg("bob", 2_000, "oracle: new question"));
    assert_eq!(watch.observe(&chat, true, 2_000), vec![1]);
    // Re-delivery of the same view answers nothing.
    assert!(watch.observe(&chat, true, 2_500).is_empty());
}

#[test]
fn own_lines_actions_and_plain_chat_never_trigger() {
    let me = UserId::new("oracle");
    let mut watch = ChatWatch::new(me);
    assert!(watch.observe(&[], true, 0).is_empty());
    let chat = vec![
        msg("oracle", 10, "oracle: talking to myself"),
        msg("alice", 11, &encode_action("oracle: waves")),
        msg("alice", 12, "what is the oracle: doing"),
        msg("alice", 13, "Oracle: real question"),
    ];
    assert_eq!(watch.observe(&chat, true, 20), vec![3]);
}

#[test]
fn stale_lines_are_ignored_and_the_cutoff_never_moves_back() {
    let mut watch = ChatWatch::new(UserId::new("oracle"));
    let t0 = 10 * MAX_TRIGGER_AGE_MILLIS;
    assert!(watch.observe(&[], true, t0).is_empty());
    // A line that surfaces late, from before the cutoff.
    let old = msg("alice", t0 - MAX_TRIGGER_AGE_MILLIS - 1, "oracle: late");
    assert!(
        watch
            .observe(std::slice::from_ref(&old), true, t0)
            .is_empty()
    );
    // The local clock steps back: the old line must stay dead.
    assert!(
        watch
            .observe(&[old], true, t0 - MAX_TRIGGER_AGE_MILLIS)
            .is_empty()
    );
}

/// One step of a chat history as the oracle observes it.
#[derive(Clone, Debug)]
enum Step {
    /// A new line lands at a position in the list (merges can insert
    /// mid-list).
    Append {
        pos: prop::sample::Index,
        sender: u8,
        kind: u8,
        backdate: bool,
    },
    /// The oracle pulls a view.
    Observe,
    /// Time passes (less than the trigger age, so fresh lines stay
    /// fresh until observed).
    Advance(u64),
    /// Adoption happens (idempotent after the first).
    Adopt,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (any::<prop::sample::Index>(), 0u8..3, 0u8..3, any::<bool>()).prop_map(
            |(pos, sender, kind, backdate)| Step::Append { pos, sender, kind, backdate }
        ),
        3 => Just(Step::Observe),
        1 => (0u64..60_000).prop_map(Step::Advance),
        1 => Just(Step::Adopt),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        dessplay_core::test_support::proptest_cases(256)
    ))]

    /// Every fresh question from someone else that lands after adoption
    /// is answered exactly once, by the next observation. Nothing else
    /// (history, own lines, actions, plain chat, stale lines, repeats) is
    /// ever answered.
    #[test]
    fn each_fresh_question_is_answered_exactly_once(
        history in 0usize..5,
        steps in prop::collection::vec(step(), 0..60),
    ) {
        let senders = ["oracle", "alice", "bob"];
        let mut now = 100 * MAX_TRIGGER_AGE_MILLIS;
        let mut chat: Vec<ChatMessage> = (0..history)
            .map(|i| msg("alice", now - 1_000 + i as u64, &format!("oracle: history {i}")))
            .collect();
        let mut watch = ChatWatch::new(UserId::new("oracle"));
        let mut adopted = false;
        let mut baselined = false;
        let mut serial = 0u64;
        // Lines that must be answered at the next observation.
        let mut pending: BTreeSet<ChatMessage> = BTreeSet::new();
        let mut answered: BTreeSet<ChatMessage> = BTreeSet::new();
        for step in steps {
            match step {
                Step::Append { pos, sender, kind, backdate } => {
                    serial += 1;
                    let text = match kind {
                        0 => format!("oracle: question {serial}"),
                        1 => encode_action(&format!("oracle: action {serial}")),
                        _ => format!("chatter {serial}"),
                    };
                    let at = if backdate { now - MAX_TRIGGER_AGE_MILLIS - 1 } else { now };
                    let message = msg(senders[sender as usize], at, &text);
                    let expected = baselined
                        && !backdate
                        && sender != 0
                        && kind == 0;
                    if expected {
                        pending.insert(message.clone());
                    }
                    let index = pos.index(chat.len() + 1);
                    chat.insert(index, message);
                }
                Step::Observe => {
                    let fresh = watch.observe(&chat, adopted, now);
                    if adopted {
                        baselined = true;
                    }
                    let got: BTreeSet<ChatMessage> =
                        fresh.iter().map(|&i| chat[i].clone()).collect();
                    prop_assert_eq!(got.len(), fresh.len(), "an index repeated");
                    for m in &got {
                        prop_assert!(answered.insert(m.clone()), "answered twice: {:?}", m);
                    }
                    prop_assert_eq!(&got, &pending);
                    pending.clear();
                }
                Step::Advance(ms) => now += ms,
                Step::Adopt => adopted = true,
            }
            // Keep "fresh" lines fresh until observed: a pending line
            // that aged out is no longer owed an answer.
            pending.retain(|m| m.timestamp.as_millis() + MAX_TRIGGER_AGE_MILLIS > now);
        }
    }
}

// ---- Request building ----------------------------------------------------

#[test]
fn request_carries_now_playing_position_and_a_bounded_chat_window() {
    let hash = Ed2kHash([7; 16]);
    let mut view = StateView {
        now_playing: Some(hash),
        ..StateView::default()
    };
    view.anidb_metadata.insert(
        hash,
        Some(AniDbMetadata {
            source: MetadataSource::AniDb,
            series_name: "Non Non Biyori".into(),
            series_id: None,
            episode_number: Some("5".into()),
        }),
    );
    for (user, position, at) in [("alice", 60_000, 5), ("bob", 754_000, 9)] {
        view.playback_position.insert(
            UserId::new(user),
            PlaybackPosition {
                position_millis: position,
                timestamp: SharedTimestamp::from_millis(at),
                file: hash,
            },
        );
    }
    view.playback_position.insert(
        UserId::new("carol"),
        PlaybackPosition {
            position_millis: 1,
            timestamp: SharedTimestamp::from_millis(99),
            file: Ed2kHash([1; 16]),
        },
    );
    for i in 0..60 {
        view.chat.push(msg("alice", i, &format!("line {i}")));
    }
    view.chat.push(msg("bob", 60, &encode_action("stretches")));
    view.chat
        .push(msg("bob", 61, "oracle: are there foxes on Okinawa?"));
    let req = build_request(&view, view.chat.len() - 1, []).expect("a question");
    assert_eq!(req.asker, "bob");
    assert_eq!(req.question, "are there foxes on Okinawa?");
    assert_eq!(req.series.as_deref(), Some("Non Non Biyori"));
    assert_eq!(req.episode.as_deref(), Some("5"));
    assert_eq!(req.filename, None);
    assert_eq!(
        req.position_millis,
        Some(754_000),
        "freshest report for this file"
    );
    assert_eq!(req.chat.len(), CONTEXT_LINES);
    assert_eq!(
        req.chat.last().unwrap(),
        "<bob> oracle: are there foxes on Okinawa?"
    );
    assert_eq!(req.chat[req.chat.len() - 2], "* bob stretches");
    let text = user_text(&req);
    assert!(text.contains("series: Non Non Biyori"));
    assert!(text.contains("episode: 5"));
    assert!(text.contains("filename: unknown"));
    assert!(text.contains("position in episode: about 12:34"));
    assert!(text.ends_with("bob asks: are there foxes on Okinawa?"));
    assert!(build_request(&view, 0, []).is_none(), "not a question");
}

// ---- Now-playing switches ------------------------------------------------

/// The playlist entries for `files`, built the way real ones are.
fn entries(files: &[(Ed2kHash, &str)]) -> Vec<PlaylistEntry> {
    let mut state = dessplay_core::CrdtState::new();
    for (i, (hash, filename)) in files.iter().enumerate() {
        state.push_playlist_entry(
            ActorId::SERVER,
            SharedTimestamp::from_millis(i as u64 + 1),
            NewPlaylistEntry {
                hash: *hash,
                added_by: UserId::new("alice"),
                filename: (*filename).into(),
                size_bytes: 1,
                duration_millis: None,
            },
        );
    }
    state.view().playlist
}

fn episode(series: &str, number: &str) -> Option<AniDbMetadata> {
    Some(AniDbMetadata {
        source: MetadataSource::AniDb,
        series_name: series.into(),
        series_id: None,
        episode_number: Some(number.into()),
    })
}

/// A view playing `file` since `since`, with episodes 4 and 5 known.
fn playing(file: Option<Ed2kHash>, since: u64, on_playlist: &[Ed2kHash]) -> StateView {
    let (ep4, ep5) = (Ed2kHash([4; 16]), Ed2kHash([5; 16]));
    let mut view = StateView {
        now_playing: file,
        now_playing_since: Some(SharedTimestamp::from_millis(since)),
        ..StateView::default()
    };
    view.anidb_metadata.insert(ep4, episode("Mushishi", "4"));
    view.anidb_metadata.insert(ep5, episode("Mushishi", "5"));
    let files: Vec<_> = [(ep4, "mushishi-04.mkv"), (ep5, "mushishi-05.mkv")]
        .into_iter()
        .filter(|(hash, _)| on_playlist.contains(hash))
        .collect();
    view.playlist = entries(&files);
    view
}

#[test]
fn switches_are_logged_from_and_to_after_the_baseline() {
    let (ep4, ep5) = (Ed2kHash([4; 16]), Ed2kHash([5; 16]));
    let mut log = NowPlayingLog::default();
    log.observe(&playing(None, 1, &[]), false);
    log.observe(&playing(Some(ep4), 10, &[ep4, ep5]), true);
    assert_eq!(log.changes().count(), 0, "the adopted state is baseline");
    // Episode 4 leaves the playlist while still playing: its cached
    // label keeps the filename.
    log.observe(&playing(Some(ep4), 10, &[ep5]), true);
    // The switch to 5: the "from" label comes from what the log saw
    // while 4 was still listed.
    log.observe(&playing(Some(ep5), 20, &[ep5]), true);
    log.observe(&playing(Some(ep5), 25, &[ep5]), true);
    log.observe(&playing(None, 30, &[]), true);
    let changes: Vec<_> = log.changes().cloned().collect();
    assert_eq!(
        changes,
        vec![
            NowPlayingChange {
                at: SharedTimestamp::from_millis(20),
                from: "Mushishi episode 4 (\"mushishi-04.mkv\")".into(),
                to: "Mushishi episode 5 (\"mushishi-05.mkv\")".into(),
            },
            NowPlayingChange {
                at: SharedTimestamp::from_millis(30),
                from: "Mushishi episode 5 (\"mushishi-05.mkv\")".into(),
                to: "nothing".into(),
            },
        ]
    );
}

#[test]
fn the_log_is_bounded() {
    let mut log = NowPlayingLog::default();
    for i in 0..(MAX_NOW_PLAYING_CHANGES as u64 + 5) {
        let file = Ed2kHash([(i % 2) as u8; 16]);
        log.observe(&playing(Some(file), i, &[]), true);
    }
    assert_eq!(log.changes().count(), MAX_NOW_PLAYING_CHANGES);
}

#[test]
fn switch_markers_interleave_with_the_chat_window_by_timestamp() {
    let change = |at, from: &str, to: &str| NowPlayingChange {
        at: SharedTimestamp::from_millis(at),
        from: from.into(),
        to: to.into(),
    };
    let changes = [
        change(5, "ep 2", "ep 3"),     // before the window
        change(1_070, "ep 3", "ep 4"), // inside it
        change(1_070, "x", "y"),       // same stamp: both, stable order
        change(2_000, "ep 4", "ep 5"), // after the question
    ];
    let mut view = StateView::default();
    for i in 0..60 {
        view.chat
            .push(msg("alice", 1_000 + i * 2, &format!("line {i}")));
    }
    view.chat
        .push(msg("bob", 1_200, "oracle: was that the same fox?"));
    let req = build_request(&view, view.chat.len() - 1, &changes).expect("a question");
    assert_eq!(req.chat.len(), CONTEXT_LINES + 2);
    let at = req
        .chat
        .iter()
        .position(|l| l.starts_with("---"))
        .expect("a marker");
    assert_eq!(req.chat[at - 1], "<alice> line 34", "stamped 1068");
    assert_eq!(req.chat[at], "--- now playing changed from ep 3 to ep 4");
    assert_eq!(req.chat[at + 1], "--- now playing changed from x to y");
    assert_eq!(req.chat[at + 2], "<alice> line 35", "stamped 1070");
    assert!(user_text(&req).contains("--- now playing changed from ep 3 to ep 4"));
}

/// Answers with the chat window it was given, one line per message.
struct EchoChat;

impl OracleModel for EchoChat {
    fn answer(&self, req: &OracleRequest) -> Result<String, OracleError> {
        Ok(req.chat.join("\n\n"))
    }
}

#[tokio::test]
async fn the_driver_shows_the_switch_it_watched() {
    let (ep4, ep5) = (Ed2kHash([4; 16]), Ed2kHash([5; 16]));
    let (mut oracle, mut results) = Oracle::new(UserId::new("oracle"), Arc::new(EchoChat));
    let mut view = playing(Some(ep4), 10, &[ep4, ep5]);
    view.chat.push(msg("alice", 900, "what a fox"));
    oracle.on_view(&view, true, 1_000);
    let mut view = playing(Some(ep5), 1_010, &[ep5]);
    view.chat.push(msg("alice", 900, "what a fox"));
    view.chat
        .push(msg("bob", 1_020, "oracle: what kind of fox was that?"));
    oracle.on_view(&view, true, 1_020);
    let outcome = results.recv().await.unwrap();
    assert_eq!(
        oracle.finish(outcome),
        vec![
            "<alice> what a fox",
            "--- now playing changed from Mushishi episode 4 (\"mushishi-04.mkv\") \
             to Mushishi episode 5 (\"mushishi-05.mkv\")",
            "<bob> oracle: what kind of fox was that?",
        ]
    );
}

fn request() -> OracleRequest {
    OracleRequest {
        asker: "alice".into(),
        question: "are there foxes on Okinawa?".into(),
        series: None,
        episode: None,
        filename: None,
        position_millis: None,
        chat: vec!["<alice> oracle: are there foxes on Okinawa?".into()],
    }
}

#[test]
fn body_uses_the_shared_model_explicit_effort_and_web_tools() {
    let body = build_body(&request(), &[], false);
    assert_eq!(body["model"], ANTHROPIC_MODEL);
    assert_eq!(body["output_config"]["effort"], "medium");
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert!(
        body.get("tool_choice").is_none(),
        "forced tool use is a 400"
    );
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["type"], "web_search_20260209");
    assert_eq!(tools[1]["type"], "web_fetch_20260209");
    assert!(
        body["system"]
            .as_str()
            .unwrap()
            .contains("Never reveal plot")
    );
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn continuation_and_wrap_up_keep_the_prefix_byte_identical() {
    let first = build_body(&request(), &[], false);
    let paused = [serde_json::json!({"type": "server_tool_use", "name": "web_search"})];
    let wrap = build_body(&request(), &paused, true);
    // Tools and system unchanged: only messages are appended.
    assert_eq!(first["tools"], wrap["tools"]);
    assert_eq!(first["system"], wrap["system"]);
    assert_eq!(first["messages"][0], wrap["messages"][0]);
    assert_eq!(wrap["messages"][1]["role"], "assistant");
    assert_eq!(wrap["messages"][2]["role"], "system");
    assert_eq!(wrap["messages"].as_array().unwrap().len(), 3);
}

// ---- The answer loop -----------------------------------------------------

/// A scripted transport: returns the queued replies in order and
/// records every request body.
struct Scripted {
    replies: RefCell<VecDeque<serde_json::Value>>,
    bodies: RefCell<Vec<serde_json::Value>>,
}

impl Scripted {
    fn new(replies: Vec<serde_json::Value>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            bodies: RefCell::new(Vec::new()),
        }
    }
}

impl MessagesApi for Scripted {
    fn messages(&self, body: &serde_json::Value) -> Result<serde_json::Value, ApiError> {
        self.bodies.borrow_mut().push(body.clone());
        self.replies
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| ApiError::Http("script exhausted".into()))
    }
}

fn searches(n: u32) -> Vec<serde_json::Value> {
    (0..n)
        .flat_map(|i| {
            [
                serde_json::json!({"type": "server_tool_use", "id": format!("s{i}"), "name": "web_search"}),
                serde_json::json!({"type": "web_search_tool_result", "tool_use_id": format!("s{i}")}),
            ]
        })
        .collect()
}

fn reply(stop: &str, content: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({"stop_reason": stop, "content": content})
}

#[test]
fn the_answer_is_the_trailing_text_with_citations_joined() {
    let mut content = vec![
        serde_json::json!({"type": "thinking", "thinking": ""}),
        serde_json::json!({"type": "text", "text": "Let me check. "}),
    ];
    content.extend(searches(1));
    content.push(serde_json::json!({"type": "text", "text": "Yes: "}));
    content.push(serde_json::json!({"type": "text", "text": "the Ryukyu fox?", "citations": []}));
    let api = Scripted::new(vec![reply("end_turn", content)]);
    let answer = answer(&api, &request()).unwrap();
    assert_eq!(answer.text, "Yes: the Ryukyu fox?");
    assert_eq!(answer.tool_calls, 1);
    assert_eq!(answer.requests, 1);
}

#[test]
fn a_paused_turn_is_resumed_with_its_content_verbatim() {
    let mut done = searches(1);
    done.push(serde_json::json!({"type": "text", "text": "No wild foxes."}));
    let api = Scripted::new(vec![
        reply("pause_turn", searches(2)),
        reply("end_turn", done),
    ]);
    let answer = answer(&api, &request()).unwrap();
    assert_eq!(answer.text, "No wild foxes.");
    assert_eq!(answer.tool_calls, 3);
    let bodies = api.bodies.borrow();
    assert_eq!(bodies.len(), 2);
    assert_eq!(
        bodies[1]["messages"][1]["content"],
        serde_json::json!(searches(2))
    );
    assert_eq!(
        bodies[1]["messages"].as_array().unwrap().len(),
        2,
        "no wrap-up yet"
    );
}

#[test]
fn refusal_truncation_and_empty_answers_are_errors() {
    let api = Scripted::new(vec![reply("refusal", vec![])]);
    assert!(matches!(
        answer(&api, &request()),
        Err(OracleError::Refused)
    ));
    let api = Scripted::new(vec![reply("max_tokens", vec![])]);
    assert!(matches!(
        answer(&api, &request()),
        Err(OracleError::Truncated)
    ));
    let api = Scripted::new(vec![reply("end_turn", searches(1))]);
    assert!(matches!(
        answer(&api, &request()),
        Err(OracleError::NoAnswer(_))
    ));
    let api = Scripted::new(vec![]);
    assert!(matches!(answer(&api, &request()), Err(OracleError::Api(_))));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        dessplay_core::test_support::proptest_cases(128)
    ))]

    /// However the model spends searches across paused turns, the loop
    /// sends no continuation once [`MAX_TOOL_CALLS`] are used. It sends
    /// exactly one wrap-up request instead, and nothing after it.
    #[test]
    fn the_search_budget_bounds_the_loop(per_reply in prop::collection::vec(0u32..8, 1..40)) {
        let replies: Vec<_> = per_reply
            .iter()
            .map(|&n| reply("pause_turn", searches(n)))
            .collect();
        let api = Scripted::new(replies);
        let _ = answer(&api, &request());
        let bodies = api.bodies.borrow();
        let wrap_ups: Vec<usize> = bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| b["messages"].as_array().unwrap().len() == 3)
            .map(|(i, _)| i)
            .collect();
        // Tool calls made before the last request was sent.
        let before_last: u32 = per_reply.iter().take(bodies.len() - 1).sum();
        let before_second_last: u32 = per_reply.iter().take(bodies.len().saturating_sub(2)).sum();
        prop_assert!(bodies.len() as u32 <= MAX_CONTINUATIONS + 2);
        prop_assert!(wrap_ups.len() <= 1);
        if let Some(&w) = wrap_ups.first() {
            prop_assert_eq!(w, bodies.len() - 1, "nothing follows the wrap-up");
            prop_assert!(before_last >= MAX_TOOL_CALLS);
            prop_assert!(before_second_last < MAX_TOOL_CALLS, "wrap-up came as soon as the budget ran out");
        } else {
            prop_assert!(before_last < MAX_TOOL_CALLS, "a continuation went out past the budget");
        }
    }
}

// ---- Reply splitting -----------------------------------------------------

#[test]
fn replies_split_on_paragraphs_with_whitespace_collapsed() {
    assert_eq!(
        split_reply("Yes!  The Ryukyu\nIslands have\n\nno native foxes, though."),
        vec!["Yes! The Ryukyu Islands have", "no native foxes, though."]
    );
    assert!(split_reply("  \n\n ").is_empty());
}

#[test]
fn replies_are_capped_in_messages_and_characters() {
    let many = (0..10)
        .map(|i| format!("p{i}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let lines = split_reply(&many);
    assert_eq!(lines.len(), MAX_REPLY_MESSAGES);
    assert!(lines.last().unwrap().ends_with('…'));
    let long = "x".repeat(MAX_REPLY_CHARS * 2);
    let lines = split_reply(&long);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].chars().count(), MAX_REPLY_CHARS);
    assert!(lines[0].ends_with('…'));
}

// ---- The driver ----------------------------------------------------------

struct Fake(Result<String, ()>);

impl OracleModel for Fake {
    fn answer(&self, req: &OracleRequest) -> Result<String, OracleError> {
        match &self.0 {
            Ok(prefix) => Ok(format!("{prefix} {}", req.question)),
            Err(()) => Err(OracleError::NoAnswer("scripted".into())),
        }
    }
}

#[tokio::test]
async fn the_driver_answers_in_order_one_at_a_time() {
    let (mut oracle, mut results) =
        Oracle::new(UserId::new("oracle"), Arc::new(Fake(Ok("re:".into()))));
    let mut view = StateView::default();
    oracle.on_view(&view, true, 1_000);
    view.chat.push(msg("alice", 1_000, "oracle: one"));
    view.chat.push(msg("bob", 1_001, "oracle: two"));
    oracle.on_view(&view, true, 1_001);
    let first = results.recv().await.unwrap();
    assert_eq!(first.asker, "alice");
    assert_eq!(oracle.finish(first), vec!["re: one"]);
    let second = results.recv().await.unwrap();
    assert_eq!(oracle.finish(second), vec!["re: two"]);
}

#[tokio::test]
async fn failures_post_one_apology_line() {
    let (mut oracle, mut results) = Oracle::new(UserId::new("oracle"), Arc::new(Fake(Err(()))));
    oracle.on_view(&StateView::default(), true, 1_000);
    let view = StateView {
        chat: vec![msg("alice", 1_000, "oracle: hm?")],
        ..StateView::default()
    };
    oracle.on_view(&view, true, 1_000);
    let outcome = results.recv().await.unwrap();
    assert_eq!(oracle.finish(outcome), vec![FAILURE_LINE]);
}

#[tokio::test]
async fn the_queue_is_bounded() {
    let (mut oracle, _results) = Oracle::new(UserId::new("oracle"), Arc::new(Fake(Ok("".into()))));
    oracle.on_view(&StateView::default(), true, 1_000);
    let view = StateView {
        chat: (0..10)
            .map(|i| msg("alice", 1_000 + i, &format!("oracle: q{i}")))
            .collect(),
        ..StateView::default()
    };
    oracle.on_view(&view, true, 1_010);
    // One went in flight immediately; the queue holds the next few.
    assert!(oracle.in_flight);
    assert_eq!(oracle.queue.len(), MAX_QUEUE);
}
