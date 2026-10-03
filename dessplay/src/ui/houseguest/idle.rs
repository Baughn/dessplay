//! What the houseguest may know about the client: a read-only summary
//! `Ui::idle_view` builds after each draw. Nothing flows the other way.

use std::time::Duration;

use tuirealm::ratatui::layout::Rect;

/// Why the client is not idle right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    /// The group is playing (a resident stays anyway, keeping out of the
    /// focused pane).
    Playing,
    /// A modal, the layout tools, or a work overlay covers the panes.
    Overlay,
    /// A held chat selection (the user is mid-copy; a resident stays,
    /// keeping out of the chat pane and the focused pane).
    Selection,
}

/// Changes whenever a chat or IRC line arrives (or history is compacted,
/// which is equally worth a look): any change is a look. A source's
/// count rising is a line from it, which asks her something when its
/// newest line does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatMark {
    /// Synced chat messages.
    pub synced: usize,
    /// Shared-clock stamp of the newest synced message.
    pub newest: Option<u64>,
    /// The newest synced message asks something (see [`asks`]).
    pub synced_asks: bool,
    /// Local IRC lines received so far.
    pub irc: usize,
    /// The newest IRC line asks something (see [`asks`]).
    pub irc_asks: bool,
}

impl ChatMark {
    /// Whether what arrived since `before` asks her something: some
    /// source's count rose, and each that rose has a newest line that
    /// asks (a compaction alone, or a question beside a plain line, is
    /// only a look).
    pub fn asks_since(&self, before: &ChatMark) -> bool {
        let synced = self.synced > before.synced;
        let irc = self.irc > before.irc;
        (synced || irc) && (!synced || self.synced_asks) && (!irc || self.irc_asks)
    }
}

/// Whether a chat line asks something: it ends in a question mark,
/// trailing whitespace aside.
pub fn asks(text: &str) -> bool {
    text.trim_end().ends_with('?')
}

/// The chat log scrolled back from the newest line: its bottom border is
/// an accordion (design.md, Chat scrollback accordion).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrollback {
    /// The accordion's cells (the border between its corners).
    pub accordion: Rect,
    /// Chat and IRC messages that arrived below since the log stopped
    /// following them.
    pub unseen: usize,
}

/// The client as the houseguest sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdleView {
    /// How long the client must be idle before she arrives; `None` when
    /// the setting is off.
    pub delay: Option<Duration>,
    /// Why the client isn't idle, if it isn't.
    pub busy: Option<Busy>,
    /// Resident Osaka: local input doesn't send her away, playback
    /// doesn't keep her out, and she keeps clear of the focused pane.
    pub resident: bool,
    /// The focused pane (resident only). While the client is in use
    /// (playing, a held selection, or local input within the idle delay)
    /// the guest protects it: she and her things rain out of it.
    pub focus: Option<Rect>,
    /// Chat arrivals, for "stop and look".
    pub chat_mark: ChatMark,
    /// The chat pane, which she turns toward when a message arrives.
    pub chat: Rect,
    /// The chat log is scrolled back (she pokes its accordion when
    /// messages go unseen).
    pub scrollback: Option<Scrollback>,
    /// Rectangles her body and bubbles never cover: the chat input, the
    /// Player Status block, the keybinding bar, inline images, and the
    /// scrollback accordion (she may stand on it) — and, for a resident
    /// while someone selects, the chat.
    pub protected: Vec<Rect>,
    /// The quiet panes she may furnish, as drawn (borders included).
    pub nooks: Vec<(super::Nook, Rect)>,
    /// Whether the terminal renders true colour.
    pub truecolor: bool,
}

impl IdleView {
    /// Whether the setting allows a visit and nothing keeps her away: a
    /// resident only minds overlays.
    pub fn open(&self) -> bool {
        self.delay.is_some()
            && match self.busy {
                None => true,
                Some(Busy::Overlay) => false,
                Some(Busy::Playing | Busy::Selection) => self.resident,
            }
    }
}

/// `rect` grown by `margin` cells on every side (saturating at the
/// screen origin); an empty rectangle stays empty.
pub fn grow(rect: Rect, margin: u16) -> Rect {
    if rect.is_empty() {
        return rect;
    }
    let x = rect.x.saturating_sub(margin);
    let y = rect.y.saturating_sub(margin);
    Rect::new(
        x,
        y,
        rect.right().saturating_add(margin) - x,
        rect.bottom().saturating_add(margin) - y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What arrived asks her something only when some source's count
    /// rose and every source that rose has a question as its newest
    /// line: a compaction alone (or the newest stamp changing) doesn't,
    /// nor does a question beside a plain line from the other source.
    #[test]
    fn a_question_is_what_arrived_asking() {
        let before = ChatMark {
            synced: 5,
            newest: Some(10),
            synced_asks: true,
            irc: 2,
            irc_asks: true,
        };
        let mark = |synced, synced_asks, irc, irc_asks| ChatMark {
            synced,
            newest: Some(11),
            synced_asks,
            irc,
            irc_asks,
        };
        assert!(mark(6, true, 2, false).asks_since(&before));
        assert!(mark(5, false, 3, true).asks_since(&before));
        assert!(mark(6, true, 3, true).asks_since(&before));
        assert!(!mark(6, false, 2, true).asks_since(&before));
        assert!(
            !mark(6, true, 3, false).asks_since(&before),
            "beside a plain line"
        );
        assert!(!mark(4, true, 2, true).asks_since(&before), "compacted");
        assert!(!mark(5, true, 2, true).asks_since(&before), "nothing new");
    }
}
