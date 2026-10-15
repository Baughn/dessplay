//! When the UI asks the session for a still of the film for the
//! houseguest's TV (phase 5c D7; design.md, Houseguest).
//!
//! While she's on her way to her TV's programme or watching it
//! ([`Guest::tv_wants_picture`]), and this client holds the now-playing
//! file, the UI asks for a still, then again once a minute has passed
//! since it last asked (a failed answer counts: a frame that wouldn't
//! come isn't asked for again at once). A question the session couldn't
//! put to the player at all ([`TvAnswer::NotAsked`]: the player not yet
//! showing the file's real video, or its last frame still being read)
//! costs no minute: it's asked again after [`NOT_ASKED_MS`]. One
//! question is out at a time, numbered, given up after [`BACKSTOP_MS`]
//! (a dead player drops the command silently); an answer to any other
//! question is ignored. A good still of the file still held goes to her
//! TV, which shows it from its next paint; a failure keeps what she
//! shows. When the file changes or stops being held, her stills go.
//!
//! Pure: the caller passes the time, the file held and the guest, and
//! sends the question [`TvFeed::turn`] gives it. The UI loop and the
//! houseguest's film tests run the same two calls ([`TvFeed::turn`],
//! [`TvFeed::deliver`]).

use std::time::Duration;

use dessplay_core::types::Ed2kHash;

use super::houseguest::{Guest, TvPicture};

/// A still is asked for again once this long has passed since the last
/// question (the user's "about once a minute").
pub const REFRESH_MS: u64 = 60_000;
/// A question unanswered this long is given up (the session's poll gives
/// mpv 2 s).
pub const BACKSTOP_MS: u64 = 3_000;
/// A question that couldn't go (the session's queue full, or closed)
/// waits this long to try again, so the loop doesn't spin on it.
pub const RETRY_MS: u64 = 250;
/// A question the session didn't put to the player
/// ([`TvAnswer::NotAsked`]) is asked again after this long: no frame was
/// taken, so no minute is spent, and the first still of an episode
/// comes within seconds of the player showing it.
pub const NOT_ASKED_MS: u64 = 5_000;

/// A question for a still of `file`, numbered (`seq`) so its answer is
/// told from a given-up question's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TvAsk {
    /// Which question (the feed's count).
    pub seq: u64,
    /// The file the still must be of.
    pub file: Ed2kHash,
}

/// The session's answer to a [`TvAsk`].
pub enum TvAnswer {
    /// A good still.
    Still(TvPicture),
    /// The player was asked, and no good frame came (none within the
    /// poll, or a black or flat one). Counts as a question.
    Failed,
    /// The player wasn't asked: no player or slot, the file not held, the
    /// player not showing its real video yet, or the slot's last frame
    /// still being read. Costs no minute.
    NotAsked,
}

/// How a question's send went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// On its way.
    Gone,
    /// The queue was full.
    Full,
    /// The session is gone.
    Closed,
}

/// The UI's side of the TV's film: what it holds, and when it last
/// asked.
#[derive(Debug, Default)]
pub struct TvFeed {
    /// The now-playing file this client holds, which stills are of.
    file: Option<Ed2kHash>,
    /// When it last asked (a question that wasn't put to the player
    /// doesn't count).
    asked: Option<u64>,
    /// The question out now (one at a time), and when it went.
    in_flight: Option<(TvAsk, u64)>,
    /// Not before then: a question that couldn't go, or wasn't asked.
    retry: Option<u64>,
    /// The last question's number.
    seq: u64,
}

impl TvFeed {
    /// The UI loop's turn: follow the now-playing file this client holds
    /// (`held`; a change clears her stills), then `send` the question
    /// due, if any. Returns whether the session is gone (`send` said
    /// [`Sent::Closed`]): the loop should end.
    pub fn turn(
        &mut self,
        guest: &mut Guest,
        held: Option<Ed2kHash>,
        now: u64,
        send: impl FnOnce(TvAsk) -> Sent,
    ) -> bool {
        if self.follow(held) {
            guest.set_tv_picture(None);
        }
        let Some(ask) = self.due(now, guest.tv_wants_picture()) else {
            return false;
        };
        match send(ask) {
            Sent::Gone => {
                tracing::trace!(seq = ask.seq, file = %ask.file, "tv feed: asked for a still");
                self.sent(ask, now);
                false
            }
            Sent::Full => {
                self.unsent(now);
                false
            }
            Sent::Closed => {
                self.unsent(now);
                true
            }
        }
    }

    /// The session's `answer` to `ask`: a good still of the file held
    /// now goes to her TV; anything else leaves what it shows. Returns
    /// whether a still went to her.
    pub fn deliver(&mut self, guest: &mut Guest, ask: TvAsk, answer: TvAnswer, now: u64) -> bool {
        let Some(picture) = self.answered(ask, answer, now) else {
            return false;
        };
        guest.set_tv_picture(Some(picture));
        true
    }

    /// Follow the now-playing file this client holds (`None`: none, or
    /// not held). Returns whether it changed: her TV's stills are of
    /// another film, or none, and go. A new film is asked for at once.
    fn follow(&mut self, held: Option<Ed2kHash>) -> bool {
        if held == self.file {
            return false;
        }
        tracing::trace!(?held, "tv feed: now-playing file held changed");
        self.file = held;
        self.asked = None;
        true
    }

    /// The question to ask now, if any: she `wants` one, a file is held,
    /// no question is out, and none went in the last [`REFRESH_MS`]. A
    /// question out longer than [`BACKSTOP_MS`] is given up first. Call
    /// [`TvFeed::sent`] once it has gone.
    fn due(&mut self, now: u64, wants: bool) -> Option<TvAsk> {
        if let Some((ask, at)) = self.in_flight
            && now.saturating_sub(at) >= BACKSTOP_MS
        {
            tracing::debug!(
                seq = ask.seq,
                "tv feed: no answer from the session; given up"
            );
            self.in_flight = None;
        }
        let file = self.file?;
        if self.retry.is_some_and(|at| now < at) {
            return None;
        }
        let rested = self
            .asked
            .is_none_or(|at| now.saturating_sub(at) >= REFRESH_MS);
        (wants && self.in_flight.is_none() && rested).then_some(TvAsk {
            seq: self.seq + 1,
            file,
        })
    }

    /// The question [`TvFeed::due`] gave went out at `now`.
    fn sent(&mut self, ask: TvAsk, now: u64) {
        self.seq = ask.seq;
        self.asked = Some(now);
        self.in_flight = Some((ask, now));
        self.retry = None;
    }

    /// The question [`TvFeed::due`] gave couldn't go at `now` (the
    /// session's queue full, or closed): it stays due, from [`RETRY_MS`]
    /// on.
    fn unsent(&mut self, now: u64) {
        self.retry = Some(now + RETRY_MS);
    }

    /// The session's answer to `ask`: the still for her TV, if it's good
    /// and of the file held now (a failure, or a still of a file since
    /// changed, gives none, and her TV keeps what it shows). An answer
    /// to a question other than the one out (given up) is ignored. A
    /// question the player wasn't asked is asked again after
    /// [`NOT_ASKED_MS`], its minute not spent.
    fn answered(&mut self, ask: TvAsk, answer: TvAnswer, now: u64) -> Option<TvPicture> {
        if self.in_flight.map(|(out, _)| out) != Some(ask) {
            tracing::trace!(
                seq = ask.seq,
                "tv feed: an answer to a question given up; ignored"
            );
            return None;
        }
        self.in_flight = None;
        match answer {
            TvAnswer::Still(picture) if Some(picture.file()) == self.file => Some(picture),
            TvAnswer::Still(_) => {
                tracing::debug!("tv feed: a still of a file no longer held; dropped");
                None
            }
            TvAnswer::Failed => None,
            TvAnswer::NotAsked => {
                self.asked = None;
                self.retry = Some(now + NOT_ASKED_MS);
                None
            }
        }
    }

    /// How long until [`TvFeed::due`] could next say to ask, while she
    /// `wants` a still (the UI loop wakes for it: she may sleep through a
    /// long watch), or until a question out is given up.
    pub fn wake_in(&self, now: u64, wants: bool) -> Option<Duration> {
        let backstop = self.in_flight.map(|(_, at)| at + BACKSTOP_MS);
        let refresh = self
            .file
            .filter(|_| wants && self.in_flight.is_none())
            .map(|_| {
                let rested = self.asked.map_or(now, |at| at + REFRESH_MS);
                rested.max(self.retry.unwrap_or(0))
            });
        [backstop, refresh]
            .into_iter()
            .flatten()
            .min()
            .map(|at| Duration::from_millis(at.saturating_sub(now)))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const A: Ed2kHash = Ed2kHash([1; 16]);
    const B: Ed2kHash = Ed2kHash([2; 16]);

    fn still(file: Ed2kHash) -> TvAnswer {
        TvAnswer::Still(super::super::houseguest::film::test_picture(file, 0))
    }

    /// Ask and send at once, as the UI loop does.
    fn ask(feed: &mut TvFeed, now: u64, wants: bool) -> Option<Ed2kHash> {
        let due = feed.due(now, wants);
        if let Some(ask) = due {
            feed.sent(ask, now);
        }
        due.map(|ask| ask.file)
    }

    /// Answer the question out.
    fn answer(feed: &mut TvFeed, answer: TvAnswer, now: u64) -> Option<TvPicture> {
        let (ask, _) = feed.in_flight.expect("a question out");
        feed.answered(ask, answer, now)
    }

    /// One question as she heads for her TV, none more while it's out or
    /// within the minute (her switch-on reuses the still), then one a
    /// minute while she watches; none while she doesn't want one, or
    /// while no file is held.
    #[test]
    fn one_question_a_minute_while_she_wants_one() {
        let mut feed = TvFeed::default();
        assert_eq!(ask(&mut feed, 0, true), None, "no file held");
        assert!(feed.follow(Some(A)));
        assert_eq!(ask(&mut feed, 1_000, false), None, "she doesn't want one");
        assert_eq!(ask(&mut feed, 2_000, true), Some(A));
        assert_eq!(ask(&mut feed, 2_500, true), None, "one out");
        assert!(answer(&mut feed, still(A), 2_600).is_some());
        for t in [3_000, 30_000, 61_999] {
            assert_eq!(ask(&mut feed, t, true), None, "{t}: within the minute");
        }
        assert_eq!(ask(&mut feed, 62_000, true), Some(A), "a minute on");
        assert!(!feed.follow(Some(A)), "the same file");
    }

    /// A failure counts as a question: no new one for a minute, and her
    /// TV keeps what it shows (no still to give, nothing cleared).
    #[test]
    fn failures_are_throttled_and_keep_the_still() {
        let mut feed = TvFeed::default();
        feed.follow(Some(A));
        assert_eq!(ask(&mut feed, 0, true), Some(A));
        assert!(answer(&mut feed, TvAnswer::Failed, 50).is_none());
        assert_eq!(ask(&mut feed, 100, true), None);
        assert_eq!(ask(&mut feed, 59_999, true), None);
        assert_eq!(ask(&mut feed, 60_000, true), Some(A));
    }

    /// A question the player wasn't asked (not yet showing the file's
    /// real video: the placeholder, or the episode before, up) spends no
    /// minute: it's asked again a few seconds on, so an episode's first
    /// still isn't a minute late (step 12b review).
    #[test]
    fn a_question_not_put_to_the_player_is_asked_again_soon() {
        let mut feed = TvFeed::default();
        feed.follow(Some(A));
        assert_eq!(ask(&mut feed, 0, true), Some(A));
        assert!(answer(&mut feed, TvAnswer::NotAsked, 10).is_none());
        assert_eq!(
            feed.wake_in(10, true),
            Some(Duration::from_millis(NOT_ASKED_MS))
        );
        assert_eq!(ask(&mut feed, NOT_ASKED_MS + 9, true), None);
        assert_eq!(ask(&mut feed, NOT_ASKED_MS + 10, true), Some(A));
        assert!(answer(&mut feed, still(A), NOT_ASKED_MS + 200).is_some());
        assert_eq!(
            ask(&mut feed, NOT_ASKED_MS + 30_000, true),
            None,
            "a question put to the player spends its minute"
        );
    }

    /// A question unanswered for 3 s is given up; another goes only when
    /// the minute allows, and the given-up one's late answer is ignored
    /// (it can't close the newer question, nor reach her TV).
    #[test]
    fn an_unanswered_question_is_given_up() {
        let mut feed = TvFeed::default();
        feed.follow(Some(A));
        assert_eq!(ask(&mut feed, 0, true), Some(A));
        let (first, _) = feed.in_flight.unwrap();
        assert_eq!(
            feed.wake_in(1_000, true),
            Some(Duration::from_millis(2_000))
        );
        assert_eq!(ask(&mut feed, 2_999, true), None);
        assert_eq!(
            ask(&mut feed, 3_000, true),
            None,
            "given up, but within the minute"
        );
        assert!(feed.answered(first, still(A), 3_500).is_none(), "late");
        assert_eq!(
            feed.wake_in(3_500, true),
            Some(Duration::from_millis(56_500))
        );
        assert_eq!(ask(&mut feed, 60_000, true), Some(A));
        assert!(feed.answered(first, still(A), 60_100).is_none(), "late");
        assert!(feed.in_flight.is_some(), "the newer question is still out");
        assert!(answer(&mut feed, still(A), 60_200).is_some());
    }

    /// A change of film clears her stills and asks at once; a still of
    /// the film before, arriving late, is dropped; with no file held,
    /// nothing is asked.
    #[test]
    fn a_new_film_clears_and_asks_afresh() {
        let mut feed = TvFeed::default();
        feed.follow(Some(A));
        assert_eq!(ask(&mut feed, 0, true), Some(A));
        assert!(feed.follow(Some(B)), "the film changed: her stills go");
        assert!(answer(&mut feed, still(A), 300).is_none(), "late, of A");
        assert_eq!(ask(&mut feed, 500, true), Some(B), "asked at once");
        assert!(answer(&mut feed, still(B), 700).is_some());
        assert!(feed.follow(None), "no longer held");
        assert_eq!(ask(&mut feed, 90_000, true), None);
        assert_eq!(feed.wake_in(90_000, true), None);
    }

    /// A question that couldn't go stays due, but not before a short
    /// wait (the loop wakes for it, rather than spinning).
    #[test]
    fn a_question_that_couldnt_go_waits_a_moment() {
        let mut feed = TvFeed::default();
        feed.follow(Some(A));
        let ask_a = feed.due(0, true).unwrap();
        assert_eq!(ask_a.file, A);
        feed.unsent(0);
        assert_eq!(feed.wake_in(0, true), Some(Duration::from_millis(RETRY_MS)));
        assert_eq!(feed.due(RETRY_MS - 1, true), None);
        assert_eq!(ask(&mut feed, RETRY_MS, true), Some(A));
        assert_eq!(ask(&mut feed, RETRY_MS + 100, true), None);
    }

    /// While she wants a still, the loop wakes when the next is due.
    #[test]
    fn the_loop_wakes_when_the_next_still_is_due() {
        let mut feed = TvFeed::default();
        assert_eq!(feed.wake_in(0, true), None, "no file");
        feed.follow(Some(A));
        assert_eq!(feed.wake_in(0, true), Some(Duration::ZERO));
        assert_eq!(feed.wake_in(0, false), None);
        ask(&mut feed, 0, true);
        answer(&mut feed, TvAnswer::Failed, 0);
        assert_eq!(
            feed.wake_in(10_000, true),
            Some(Duration::from_millis(50_000))
        );
        assert_eq!(feed.wake_in(10_000, false), None);
    }
}
