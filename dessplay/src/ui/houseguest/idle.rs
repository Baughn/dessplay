//! What the houseguest may know about the client: a read-only summary
//! `Ui::idle_view` builds after each draw. Nothing flows the other way.

use std::time::Duration;

use tuirealm::ratatui::layout::Rect;

/// Why the client is not idle right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    /// The group is playing (a resident stays anyway).
    Playing,
    /// A modal, the layout tools, or a work overlay covers the panes.
    Overlay,
    /// A held chat selection (the user is mid-copy; a resident keeps
    /// out of the chat pane instead).
    Selection,
}

/// Changes whenever a chat or IRC line arrives (or history is compacted,
/// which is equally worth a look). Compared for equality only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatMark {
    /// Synced chat messages.
    pub synced: usize,
    /// Shared-clock stamp of the newest synced message.
    pub newest: Option<u64>,
    /// Local IRC lines.
    pub irc: usize,
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
    /// The focused pane (resident only), also in `protected`: she and
    /// her things rain out of it the moment it's focused.
    pub focus: Option<Rect>,
    /// Chat arrivals, for "stop and look".
    pub chat_mark: ChatMark,
    /// The chat pane, which she turns toward when a message arrives.
    pub chat: Rect,
    /// Rectangles her body and bubbles never cover: the chat input, the
    /// Player Status block, the keybinding bar, and inline images (and,
    /// for a resident, the focused pane).
    pub protected: Vec<Rect>,
    /// The quiet panes she may furnish, as drawn (borders included).
    pub nooks: Vec<(super::Nook, Rect)>,
    /// Whether the terminal renders true colour.
    pub truecolor: bool,
}

impl IdleView {
    /// Whether the client is idle and the setting allows a visit.
    pub fn open(&self) -> bool {
        self.delay.is_some() && self.busy.is_none()
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
