//! Her record: what outlives a visit — her home, and the seed and count
//! her visits are drawn from. Local and never synced, the same tier as
//! the layout sizes (proposal, Persistence).
//!
//! JSON with a version; fields default when missing, and pieces or rooms
//! this build doesn't know are skipped rather than failing the record.
//!
//! Where each piece stands is in `anchors` (its strip and anchor), a
//! field older builds ignore; for them, `rooms` (a pane per old room
//! kind) and each piece's `at` are still written, so they keep every
//! piece, placed by its share of the way along its room's pane. A record
//! an older build saved has no `anchors`: its pieces are anchored from
//! `rooms` and `at` again.
//!
//! The pieces she hasn't settled yet (deliveries, standing where they
//! came in) are listed in `unsettled`, written only when there are any;
//! a record without it has every piece settled.
//!
//! Her clock (game minutes since Monday 16:00 of game day 0) and her idle
//! and pity counters come after it, each written only when it isn't
//! zero and read leniently: a value this build can't take (not a whole
//! number, negative, or absurdly large) reads as zero without failing
//! the record. A record without them is at Monday 16:00 with nothing
//! counted, which is where every record from before them starts.
//!
//! The real date she last delivered her calendar's owed entry on comes
//! next, as `"YYYY-MM-DD"`, written only once there is one and read
//! leniently: a value this build can't read as a date reads as none
//! (the day's entry is owed again), without failing the record.
//!
//! The rare things she has shown come next, by their stable ids
//! ([`super::rarity::RARES`]), written only once there are any and read
//! leniently, entry by entry: an id this build doesn't know (a later
//! build's), or anything that isn't an id, is skipped, and the rest of
//! the record still reads.
//!
//! Her external door's wall (a strip and its side) comes next, written
//! only once she has one, and read leniently: anything this build can't
//! read as a wall reads as none, without failing the record (the next
//! frame chooses again, deterministically from her home and the panes).
//! An older build drops it when it saves the record again, and lays the
//! pieces on that strip against the raw wall: those anchored from the
//! door's wall stand up to 6 columns nearer it, in her door's space,
//! until a newer build chooses the wall again (shares, `at`, are always
//! of the raw strip, so nothing is lost).
//!
//! Whether her wall clock has been sent comes last, written only once it
//! has, and read leniently: anything but `true` reads as not yet.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::room::{Anchor, DoorWall, Furniture, Home, Nook, Prop, Side, Strip};
use super::sprite::Facing;

/// The format this build writes.
const VERSION: u32 = 1;
/// Its key in local settings storage.
// Not "houseguest": that key is her arrival setting (config.rs).
const KEY: &str = "houseguest_ledger";

/// Everything that outlives a visit.
#[derive(Clone, Debug, PartialEq)]
pub struct Ledger {
    /// Every visit's randomness derives from this.
    pub(super) master_seed: u64,
    /// Visits so far.
    pub(super) visits: u64,
    /// Her home.
    pub(super) home: Home,
    /// Bought and not yet delivered.
    pub(super) ordered: Option<Furniture>,
    /// The visit she last bought something on.
    pub(super) bought_on: u64,
    /// Her clock: game minutes since `START`, Monday 16:00 of game day
    /// 0 (part of the format; it never changes). It runs only while
    /// dessplay is open and she has met you (phase 5b D1).
    pub(super) clock: u64,
    /// Real minutes the client has stood idle with her gate open, present
    /// or not, once she has met you (her pity counter, phase 5b D6).
    pub(super) idle_min: u64,
    /// `idle_min` when she last showed something rare for the first time.
    pub(super) rare_at: u64,
    /// `idle_min` when she last showed something legendary for the first
    /// time.
    pub(super) legend_at: u64,
    /// The real date she last delivered what her calendar owed her on
    /// (phase 5b D5): owed once a day, so not again that date.
    pub(super) calendar_on: Option<NaiveDate>,
    /// The rare things she has shown, by their stable ids, in the order
    /// she first showed them (phase 5b D6): each once, each one this
    /// build knows.
    pub(super) seen: Vec<String>,
    /// Her wall clock has been sent (phase 5b D7): a one-time gift on her
    /// doorstep, never again, even once it's gone.
    pub(super) clock_sent: bool,
}

impl Ledger {
    /// A first meeting.
    pub fn new(master_seed: u64) -> Self {
        Self {
            master_seed,
            visits: 0,
            home: Home::default(),
            ordered: None,
            bought_on: 0,
            clock: 0,
            idle_min: 0,
            rare_at: 0,
            legend_at: 0,
            calendar_on: None,
            seen: Vec::new(),
            clock_sent: false,
        }
    }

    /// A home she has visited once, her clock at `at` (tests: a scene at
    /// a time of her day, her pieces added before it begins).
    #[cfg(test)]
    pub(crate) fn new_at(master_seed: u64, at: super::routine::GameTime) -> Self {
        Self {
            visits: 1,
            clock: at.minutes(),
            ..Self::new(master_seed)
        }
    }

    /// The rare scripts she has shown.
    pub(super) fn seen_scripts(&self) -> Vec<super::script::ScriptId> {
        self.seen
            .iter()
            .filter_map(|key| super::rarity::by_key(key))
            .collect()
    }

    /// She has shown the rare script her ledger records as `key`, of
    /// `tier`: seen from now on, and, the first time, her pity for its
    /// tier starts again from her idle minutes now. Returns whether the
    /// record changed (not for one she'd shown already, nor for an
    /// ungated tier's, which has no pity).
    pub(super) fn mark_seen(&mut self, key: &str, tier: super::rarity::Rarity) -> bool {
        use super::rarity::Rarity;
        if self.seen.iter().any(|k| k == key) {
            return false;
        }
        match tier {
            Rarity::Rare => self.rare_at = self.idle_min,
            Rarity::Legendary => self.legend_at = self.idle_min,
            Rarity::Common | Rarity::Uncommon => return false,
        }
        self.seen.push(key.to_owned());
        true
    }

    /// The seed of visit number `visit` (the first is the master seed
    /// itself).
    pub(super) fn visit_seed(&self, visit: u64) -> u64 {
        self.master_seed ^ visit.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    /// Read from JSON; `Err` for text that isn't a record this build can
    /// read (malformed, not an object, or another version).
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let raw: Raw = object(value).ok_or("not a JSON object")??;
        if raw.version != VERSION {
            return Err(format!("unsupported version {}", raw.version));
        }
        let mut rooms: Vec<(RoomKind, Nook)> = Vec::new();
        for (kind, nook) in raw
            .rooms
            .into_iter()
            .filter_map(|v| serde_json::from_value::<(RoomKind, Nook)>(v).ok())
        {
            if !rooms.iter().any(|&(k, n)| k == kind || n == nook) {
                rooms.push((kind, nook));
            }
        }
        let anchors: Vec<SavedAnchor> = raw
            .anchors
            .into_iter()
            .filter_map(|v| object::<SavedAnchor>(v)?.ok())
            .collect();
        let unsettled: Vec<Furniture> = raw
            .unsettled
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Furniture>(v).ok())
            .collect();
        let mut home = Home::default();
        for prop in raw
            .props
            .into_iter()
            .filter_map(|v| object::<SavedProp>(v)?.ok())
        {
            if home.owns(prop.item) {
                continue;
            }
            let anchored = anchors.iter().find(|a| a.item == prop.item);
            let room = rooms.iter().find(|&&(k, _)| k == RoomKind::of(prop.item));
            // A piece with neither a strip nor its room's pane on record
            // is dropped: it could never be placed.
            let Some(nook) = room.map(|&(_, nook)| nook).or_else(|| {
                anchored.map(|a| {
                    let Strip::Bottom(nook) = a.strip;
                    nook
                })
            }) else {
                continue;
            };
            let mut piece = Prop {
                boxed: prop.boxed,
                settled: !unsettled.contains(&prop.item),
                ..Prop::new(prop.item, nook, prop.at, prop.facing)
            };
            if let Some(a) = anchored {
                piece.strip = a.strip;
                piece.anchor = a.anchor;
            }
            home.props.push(piece);
        }
        home.door = raw
            .door
            .and_then(|v| serde_json::from_value::<DoorWall>(v).ok());
        let ordered = raw
            .ordered
            .and_then(|v| serde_json::from_value::<Furniture>(v).ok())
            .filter(|&item| !home.owns(item));
        Ok(Self {
            master_seed: raw.master_seed,
            visits: raw.visits,
            home,
            ordered,
            bought_on: raw.bought_on,
            clock: minutes(raw.clock),
            idle_min: minutes(raw.idle_min),
            rare_at: minutes(raw.rare_at),
            legend_at: minutes(raw.legend_at),
            calendar_on: raw.calendar_on.as_ref().and_then(day),
            seen: seen(raw.seen.as_ref()),
            clock_sent: raw
                .clock_sent
                .as_ref()
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// As JSON.
    pub fn to_json(&self) -> String {
        let raw = Saved {
            version: VERSION,
            master_seed: self.master_seed,
            visits: self.visits,
            rooms: rooms(&self.home),
            props: self
                .home
                .props
                .iter()
                .map(|p| SavedProp {
                    item: p.item,
                    at: p.at,
                    facing: p.facing,
                    boxed: p.boxed,
                })
                .collect(),
            anchors: self
                .home
                .props
                .iter()
                .map(|p| SavedAnchor {
                    item: p.item,
                    strip: p.strip,
                    anchor: p.anchor,
                })
                .collect(),
            ordered: self.ordered,
            bought_on: self.bought_on,
            unsettled: self
                .home
                .props
                .iter()
                .filter(|p| !p.settled)
                .map(|p| p.item)
                .collect(),
            clock: self.clock,
            idle_min: self.idle_min,
            rare_at: self.rare_at,
            legend_at: self.legend_at,
            calendar_on: self.calendar_on.map(|d| d.format(DATE).to_string()),
            seen: self.seen.clone(),
            door: self.home.door,
            clock_sent: self.clock_sent,
        };
        serde_json::to_string(&raw).unwrap_or_default()
    }

    /// Load from local storage: `Ok(None)` when she has never visited.
    pub fn load(storage: &crate::storage::Storage) -> crate::storage::Result<Option<Self>> {
        storage
            .setting(KEY)?
            .map(|text| {
                Self::from_json(&text).map_err(|e| {
                    crate::storage::StorageError::Corrupt(format!("houseguest ledger: {e}"))
                })
            })
            .transpose()
    }

    /// Save to local storage.
    pub fn save(&self, storage: &crate::storage::Storage) -> crate::storage::Result<()> {
        storage.set_setting(KEY, Some(&self.to_json()))
    }

    /// What the record says, for `dessplay --dump` (read from the saved
    /// record, never a live guest). Her slot is judged by her routine
    /// with school out or not by the real `date` (`None`: unknown, so
    /// term time); a running client latches each game day's flag at that
    /// day's first read, so the two can differ on a day the date turned.
    pub fn summary(&self, date: Option<NaiveDate>) -> Summary {
        use super::rarity::{Pity, Rarity};
        use super::routine;
        let game = self.clock.saturating_mul(super::GAME_MINUTE_MS);
        let vacation = date.is_some_and(routine::vacation);
        let now = routine::day_time(game, vacation);
        let pity = Pity::of(self.idle_min, self.rare_at, self.legend_at);
        let tier = |tier: Rarity, since: u64, at: u64| {
            let unseen: Vec<String> = super::rarity::RARES
                .iter()
                .filter(|r| r.rarity == tier && !self.seen.iter().any(|k| k == r.key))
                .map(|r| r.key.to_owned())
                .collect();
            TierPity {
                running: !unseen.is_empty(),
                unseen,
                since_new_min: since,
                new_at_idle_min: at,
                certain_at_min: tier.bound().unwrap_or(0),
            }
        };
        Summary {
            visits: self.visits,
            master_seed: format!("{:#018x}", self.master_seed),
            clock_min: self.clock,
            game_day: now.day,
            game_time: routine::label(game),
            slot: format!("{:?}", now.slot),
            vacation,
            vacation_by: match date {
                Some(date) => format!(
                    "today's date as the client counts it (the day starts at 09:00), {date} \
                     (a running client holds each game day's flag from that day's first read)"
                ),
                None => "no real date: judged as term time".to_owned(),
            },
            idle_min: self.idle_min,
            pity: Pities {
                rare: tier(Rarity::Rare, pity.rare, self.rare_at),
                legendary: tier(Rarity::Legendary, pity.legend, self.legend_at),
            },
            seen: self.seen.clone(),
            calendar_on: self.calendar_on.map(|d| d.format(DATE).to_string()),
            clock_sent: self.clock_sent,
            door: self.home.door.map(|door| {
                let Strip::Bottom(nook) = door.strip;
                let side = match door.side {
                    Side::Left => "left",
                    Side::Right => "right",
                };
                format!("{nook:?} {side}")
            }),
            owns: self
                .home
                .props
                .iter()
                .map(|p| Owned {
                    item: format!("{:?}", p.item),
                    boxed: p.boxed,
                    settled: p.settled,
                    strip: format!("{:?}", p.strip),
                })
                .collect(),
            ordered: self.ordered.map(|item| format!("{item:?}")),
            bought_on_visit: self.bought_on,
        }
    }
}

/// Her record as `dessplay --dump` shows it ([`Ledger::summary`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Visits so far.
    pub visits: u64,
    /// The seed every visit's randomness derives from, in hex (a 64-bit
    /// number loses precision in most JSON readers).
    pub master_seed: String,
    /// Her clock: game minutes since Monday 16:00 of game day 0.
    pub clock_min: u64,
    /// The game day her clock is on (day 0 is the Monday it started).
    pub game_day: u64,
    /// Her game time, weekday and `HH:MM` ("Mon 16:05").
    pub game_time: String,
    /// The part of her day it is then, by her routine.
    pub slot: String,
    /// Whether school was judged out for `slot`.
    pub vacation: bool,
    /// What `vacation` was judged by.
    pub vacation_by: String,
    /// Real minutes the client has stood idle with her gate open (what
    /// her pity counts in).
    pub idle_min: u64,
    /// Her pity, for each tier that has one.
    pub pity: Pities,
    /// The rare things she has shown, by their stable ids, in the order
    /// she first showed them.
    pub seen: Vec<String>,
    /// The real date her calendar's owed entry was last delivered on.
    pub calendar_on: Option<String>,
    /// Her wall clock has been sent.
    pub clock_sent: bool,
    /// Her external door's wall, once she has one ("Users right").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door: Option<String>,
    /// The pieces she owns.
    pub owns: Vec<Owned>,
    /// Bought and not yet delivered.
    pub ordered: Option<String>,
    /// The visit she last bought something on.
    pub bought_on_visit: u64,
}

/// Her pity for each gated tier ([`Summary::pity`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pities {
    /// Rare.
    pub rare: TierPity,
    /// Legendary.
    pub legendary: TierPity,
}

/// One tier's pity: whether it runs at all, how long since she last
/// showed something new of it, and when a draw is certain to make
/// something unseen new.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TierPity {
    /// Whether pity runs for the tier: only while something of it is
    /// unseen (with nothing left to show for the first time, the
    /// counters below mean nothing).
    pub running: bool,
    /// The tier's scripts she hasn't shown yet, by stable id.
    pub unseen: Vec<String>,
    /// Real idle minutes since she last showed something of the tier for
    /// the first time (0 for a counter read ahead of `idle_min`).
    pub since_new_min: u64,
    /// `idle_min` when she last did.
    pub new_at_idle_min: u64,
    /// The pity at which a draw is certain to make something unseen of
    /// the tier new (only while it's `running`).
    pub certain_at_min: u64,
}

/// A piece she owns ([`Summary::owns`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Owned {
    /// What it is.
    pub item: String,
    /// Still in its box.
    pub boxed: bool,
    /// She has set it where it stands (a delivery hasn't yet).
    pub settled: bool,
    /// The strip (pane) it stands on.
    pub strip: String,
}

#[derive(Serialize, Deserialize)]
struct SavedProp {
    item: Furniture,
    #[serde(default)]
    at: u16,
    #[serde(default = "facing_right")]
    facing: Facing,
    #[serde(default)]
    boxed: bool,
}

fn facing_right() -> Facing {
    Facing::Right
}

/// The most a count of minutes holds, counting up in memory and as
/// read (one constant, so a count stopped at the top reads back as
/// itself): just under 2^40, far past anything real (two million
/// years), and far enough below `u64::MAX` that no arithmetic on it
/// overflows.
pub(super) const MINUTES_MAX: u64 = (1 << 40) - 1;

/// A count of minutes as read: a whole number up to [`MINUTES_MAX`], or
/// zero (missing, or a value this build can't take).
fn minutes(value: Option<serde_json::Value>) -> u64 {
    value
        .as_ref()
        .and_then(serde_json::Value::as_u64)
        .filter(|&m| m <= MINUTES_MAX)
        .unwrap_or(0)
}

/// How a date is written: `"YYYY-MM-DD"` (chrono has no serde here).
const DATE: &str = "%Y-%m-%d";

/// A date as read: a string in [`DATE`]'s form naming a real date, or
/// none (a value this build can't take).
fn day(value: &serde_json::Value) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.as_str()?, DATE).ok()
}

/// The rare things she has shown, as read: each entry a stable id this
/// build knows, once (in the order first written); anything else
/// (another build's id, a value that isn't an id, a list that isn't one)
/// skipped.
fn seen(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|key| super::rarity::by_key(key).is_some())
    {
        if !out.iter().any(|k| k == key) {
            out.push(key.to_owned());
        }
    }
    out
}

/// Whether a count is left out of the record (serde's
/// `skip_serializing_if` takes a reference).
fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Which pane each room is in, as older builds read it: the pane of its
/// first piece's strip, or, where another room has that pane, the first
/// pane none has (each room needs a pane of its own there).
fn rooms(home: &Home) -> Vec<(RoomKind, Nook)> {
    let mut out: Vec<(RoomKind, Nook)> = Vec::new();
    // A later kind claims no room's pane: older builds don't know it.
    for prop in home.props.iter().filter(|p| p.item.legacy()) {
        let kind = RoomKind::of(prop.item);
        if out.iter().any(|&(k, _)| k == kind) {
            continue;
        }
        let Strip::Bottom(nook) = prop.strip;
        let taken = |n: Nook| out.iter().any(|&(_, t)| t == n);
        let free = [nook, Nook::List, Nook::Users, Nook::Playlist]
            .into_iter()
            .find(|&n| !taken(n));
        if let Some(nook) = free {
            out.push((kind, nook));
        }
    }
    out
}

/// The rooms older builds keep pieces in, one pane each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum RoomKind {
    Living,
    Bedroom,
    Kitchen,
}

impl RoomKind {
    /// The room an older build keeps `item` in.
    fn of(item: Furniture) -> Self {
        match item {
            Furniture::Sofa
            | Furniture::Tv
            | Furniture::CatBed
            | Furniture::Plant
            | Furniture::Poster
            | Furniture::Clock
            | Furniture::Window => Self::Living,
            Furniture::Bed | Furniture::Desk | Furniture::Lamp | Furniture::Bookshelf => {
                Self::Bedroom
            }
            Furniture::Fridge => Self::Kitchen,
        }
    }
}

#[derive(Serialize)]
struct Saved {
    version: u32,
    master_seed: u64,
    visits: u64,
    rooms: Vec<(RoomKind, Nook)>,
    props: Vec<SavedProp>,
    anchors: Vec<SavedAnchor>,
    ordered: Option<Furniture>,
    bought_on: u64,
    /// Pieces she hasn't settled; left out when there are none, so a
    /// record with none reads as before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unsettled: Vec<Furniture>,
    /// Her clock and counters; each left out at zero, so a record with
    /// none reads as before.
    #[serde(skip_serializing_if = "is_zero")]
    clock: u64,
    #[serde(skip_serializing_if = "is_zero")]
    idle_min: u64,
    #[serde(skip_serializing_if = "is_zero")]
    rare_at: u64,
    #[serde(skip_serializing_if = "is_zero")]
    legend_at: u64,
    /// Left out until she has delivered a calendar entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    calendar_on: Option<String>,
    /// Left out until she has shown something rare.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    seen: Vec<String>,
    /// Her door's wall; left out until she has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    door: Option<DoorWall>,
    /// Left out until her wall clock has been sent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    clock_sent: bool,
}

/// Where a piece stands: its strip, and its anchor there once it has
/// stood there.
#[derive(Serialize, Deserialize)]
struct SavedAnchor {
    item: Furniture,
    strip: Strip,
    #[serde(default)]
    anchor: Option<Anchor>,
}

/// A record's struct from `value` only when it's a JSON object (`None`
/// otherwise; `Some(Err)` for an object that isn't one): serde reads a
/// struct from an array too, by position, and nothing writes that.
fn object<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Option<Result<T, String>> {
    value
        .is_object()
        .then(|| serde_json::from_value(value).map_err(|e| e.to_string()))
}

/// What's read, before the entries this build knows are picked out.
#[derive(Deserialize)]
struct Raw {
    version: u32,
    #[serde(default)]
    master_seed: u64,
    #[serde(default)]
    visits: u64,
    #[serde(default)]
    rooms: Vec<serde_json::Value>,
    #[serde(default)]
    props: Vec<serde_json::Value>,
    #[serde(default)]
    anchors: Vec<serde_json::Value>,
    #[serde(default)]
    ordered: Option<serde_json::Value>,
    #[serde(default)]
    bought_on: u64,
    #[serde(default)]
    unsettled: Vec<serde_json::Value>,
    #[serde(default)]
    clock: Option<serde_json::Value>,
    #[serde(default)]
    idle_min: Option<serde_json::Value>,
    #[serde(default)]
    rare_at: Option<serde_json::Value>,
    #[serde(default)]
    legend_at: Option<serde_json::Value>,
    #[serde(default)]
    calendar_on: Option<serde_json::Value>,
    #[serde(default)]
    seen: Option<serde_json::Value>,
    #[serde(default)]
    door: Option<serde_json::Value>,
    #[serde(default)]
    clock_sent: Option<serde_json::Value>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::room::Side;
    use super::super::script::ScriptId;
    use super::*;

    fn furnished() -> Ledger {
        let mut ledger = Ledger::new(42);
        ledger.visits = 7;
        ledger.ordered = Some(Furniture::Desk);
        ledger.bought_on = 6;
        for (item, at) in [(Furniture::Sofa, 0), (Furniture::Tv, 900)] {
            assert!(
                ledger
                    .home
                    .add(Prop::new(item, Nook::Users, at, Facing::Left))
            );
        }
        assert!(ledger.home.add(Prop {
            boxed: true,
            ..Prop::new(Furniture::Bed, Nook::Playlist, 500, Facing::Right)
        }));
        // The sofa and the bed have stood on their strips; the TV not yet.
        ledger.home.props[0].anchor = Some(Anchor {
            side: Side::Left,
            offset: 0,
        });
        ledger.home.props[2].anchor = Some(Anchor {
            side: Side::Right,
            offset: 7,
        });
        ledger
    }

    /// The kinds of piece the oldest builds know, as they're written.
    const OLDEST_KINDS: [&str; 8] = [
        "Sofa",
        "Tv",
        "Bed",
        "Desk",
        "Lamp",
        "Bookshelf",
        "Fridge",
        "CatBed",
    ];

    /// What an older build reads of a record: the pane of each room, and
    /// each piece in its room's pane at its share of the way along (a
    /// piece whose room has no pane is dropped, a second room in one pane
    /// too).
    fn as_an_older_build_reads(text: &str) -> Vec<(Furniture, Nook, u16)> {
        let raw: serde_json::Value = serde_json::from_str(text).unwrap();
        // Every older build refuses another version.
        assert_eq!(raw["version"], 1, "an older build can't read {text}");
        let mut rooms: Vec<(RoomKind, Nook)> = Vec::new();
        for room in raw["rooms"].as_array().unwrap() {
            let (kind, nook): (RoomKind, Nook) = serde_json::from_value(room.clone()).unwrap();
            if !rooms.iter().any(|&(k, n)| k == kind || n == nook) {
                rooms.push((kind, nook));
            }
        }
        raw["props"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|prop| {
                // It knows only the first eight kinds, by name (frozen
                // here, not this build's `legacy()`): an unknown piece is
                // skipped.
                let name = prop["item"].as_str()?;
                if !OLDEST_KINDS.contains(&name) {
                    return None;
                }
                let item = serde_json::from_value::<Furniture>(prop["item"].clone()).ok()?;
                let at = prop["at"].as_u64().unwrap() as u16;
                let &(_, nook) = rooms.iter().find(|&&(k, _)| k == RoomKind::of(item))?;
                Some((item, nook, at))
            })
            .collect()
    }

    /// The record as this build writes it, character for character: a
    /// change here is a change to what's saved, and to what older builds
    /// read.
    #[test]
    fn the_record_as_written() {
        assert_eq!(
            furnished().to_json(),
            concat!(
                r#"{"version":1,"master_seed":42,"visits":7,"#,
                r#""rooms":[["Living","Users"],["Bedroom","Playlist"]],"#,
                r#""props":[{"item":"Sofa","at":0,"facing":"Left","boxed":false},"#,
                r#"{"item":"Tv","at":900,"facing":"Left","boxed":false},"#,
                r#"{"item":"Bed","at":500,"facing":"Right","boxed":true}],"#,
                r#""anchors":[{"item":"Sofa","strip":{"Bottom":"Users"},"anchor":{"side":"Left","offset":0}},"#,
                r#"{"item":"Tv","strip":{"Bottom":"Users"},"anchor":null},"#,
                r#"{"item":"Bed","strip":{"Bottom":"Playlist"},"anchor":{"side":"Right","offset":7}}],"#,
                r#""ordered":"Desk","bought_on":6}"#,
            )
        );
    }

    /// A record from before anchors (or one an older build saved since)
    /// reads with each piece on its room's pane, to be anchored by its
    /// share of the way along when that pane is next seen.
    #[test]
    fn an_older_record_reads() {
        let text = r#"{"version":1,"master_seed":42,"visits":7,"rooms":[["Living","Users"],["Bedroom","Playlist"]],"props":[{"item":"Sofa","at":0,"facing":"Left","boxed":false},{"item":"Tv","at":900,"facing":"Left","boxed":false},{"item":"Bed","at":500,"facing":"Right","boxed":true}],"ordered":"Desk","bought_on":6}"#;
        let mut want = furnished();
        for prop in &mut want.home.props {
            prop.anchor = None;
        }
        assert_eq!(Ledger::from_json(text), Ok(want));
    }

    /// An older build keeps every piece, whichever strips they stand on:
    /// rooms sharing a pane here get a pane each there.
    #[test]
    fn an_older_build_keeps_every_piece() {
        let mut ledger = furnished();
        assert_eq!(
            as_an_older_build_reads(&ledger.to_json()),
            [
                (Furniture::Sofa, Nook::Users, 0),
                (Furniture::Tv, Nook::Users, 900),
                (Furniture::Bed, Nook::Playlist, 500),
            ]
        );
        // The bed has moved onto the living room's strip.
        ledger.home.props[2].strip = Strip::Bottom(Nook::Users);
        let read = as_an_older_build_reads(&ledger.to_json());
        assert_eq!(read.len(), 3, "{read:?}");
        assert_ne!(read[2].1, Nook::Users, "{read:?}");
        // And this build reads it back where it stands.
        assert_eq!(Ledger::from_json(&ledger.to_json()), Ok(ledger));
    }

    /// Decor is a piece older builds don't know: they skip it, and it
    /// claims no room's pane there, so every piece they do know keeps
    /// its room's.
    #[test]
    fn an_older_build_skips_decor() {
        let mut ledger = Ledger::new(3);
        for (item, nook, at) in [
            (Furniture::Poster, Nook::Users, 0),
            (Furniture::Plant, Nook::List, 1000),
            (Furniture::Sofa, Nook::Playlist, 0),
            (Furniture::Tv, Nook::Playlist, 1000),
        ] {
            assert!(ledger.home.add(Prop {
                anchor: Some(Anchor {
                    side: Side::Left,
                    offset: at / 100,
                }),
                ..Prop::new(item, nook, at, Facing::Right)
            }));
        }
        let text = ledger.to_json();
        assert_eq!(
            as_an_older_build_reads(&text),
            [
                (Furniture::Sofa, Nook::Playlist, 0),
                (Furniture::Tv, Nook::Playlist, 1000),
            ]
        );
        assert_eq!(Ledger::from_json(&text), Ok(ledger));
    }

    /// A strip this build doesn't know (a later build's) falls back to
    /// the piece's room's pane.
    #[test]
    fn an_unknown_strip_falls_back_to_the_rooms_pane() {
        let text = furnished()
            .to_json()
            .replace(r#"{"Bottom":"Playlist"}"#, r#"{"Shelf":3}"#);
        let ledger = Ledger::from_json(&text).unwrap();
        let bed = ledger.home.props[2];
        assert_eq!(bed.strip, Strip::Bottom(Nook::Playlist));
        assert_eq!(bed.anchor, None);
    }

    /// A delivery she hasn't settled is listed after everything else; it
    /// reads back unsettled, and the rest settled.
    #[test]
    fn unsettled_pieces_are_written_and_read_back() {
        let mut ledger = furnished();
        ledger.home.props[1].settled = false;
        let text = ledger.to_json();
        assert!(
            text.ends_with(r#""ordered":"Desk","bought_on":6,"unsettled":["Tv"]}"#),
            "{text}"
        );
        let read = Ledger::from_json(&text).unwrap();
        assert_eq!(
            read.home
                .props
                .iter()
                .map(|p| (p.item, p.settled))
                .collect::<Vec<_>>(),
            [
                (Furniture::Sofa, true),
                (Furniture::Tv, false),
                (Furniture::Bed, true),
            ]
        );
        assert_eq!(read, ledger);
    }

    /// A record without the list (an older build's, or one with nothing
    /// unsettled) has every piece settled.
    #[test]
    fn an_older_record_is_all_settled() {
        let text = furnished().to_json();
        assert!(!text.contains("unsettled"), "{text}");
        let ledger = Ledger::from_json(&text).unwrap();
        assert!(ledger.home.props.iter().all(|p| p.settled));
    }

    /// A later build's piece in the list is skipped; the rest still read.
    #[test]
    fn an_unknown_unsettled_piece_is_skipped() {
        let text = furnished().to_json().replace(
            r#""bought_on":6}"#,
            r#""bought_on":6,"unsettled":["Piano",{"x":1},"Bed"]}"#,
        );
        let ledger = Ledger::from_json(&text).unwrap();
        assert_eq!(
            ledger
                .home
                .props
                .iter()
                .map(|p| (p.item, p.settled))
                .collect::<Vec<_>>(),
            [
                (Furniture::Sofa, true),
                (Furniture::Tv, true),
                (Furniture::Bed, false),
            ]
        );
    }

    #[test]
    fn a_ledger_round_trips() {
        let ledger = furnished();
        assert_eq!(Ledger::from_json(&ledger.to_json()), Ok(ledger));
    }

    /// `furnished`, with her clock and counters running.
    fn timed() -> Ledger {
        Ledger {
            clock: 3 * 1440 + 125,
            idle_min: 900,
            rare_at: 400,
            legend_at: 17,
            ..furnished()
        }
    }

    /// Her clock and counters come last, after anything unsettled, and
    /// read back as written.
    #[test]
    fn the_clock_and_counters_round_trip() {
        let mut ledger = timed();
        ledger.home.props[1].settled = false;
        let text = ledger.to_json();
        assert!(
            text.ends_with(concat!(
                r#""bought_on":6,"unsettled":["Tv"],"#,
                r#""clock":4445,"idle_min":900,"rare_at":400,"legend_at":17}"#
            )),
            "{text}"
        );
        assert_eq!(Ledger::from_json(&text), Ok(ledger));
        // Each on its own, the others at zero (left out).
        for one in [
            Ledger {
                clock: 1,
                ..furnished()
            },
            Ledger {
                idle_min: 2,
                ..furnished()
            },
            Ledger {
                rare_at: 3,
                ..furnished()
            },
            Ledger {
                legend_at: 4,
                ..furnished()
            },
        ] {
            let text = one.to_json();
            assert_eq!(
                text.matches(':').count(),
                furnished().to_json().matches(':').count() + 1,
                "{text}"
            );
            assert_eq!(Ledger::from_json(&text), Ok(one));
        }
    }

    /// At zero none is written: a record without them is as before.
    #[test]
    fn a_stopped_clock_writes_nothing() {
        let text = furnished().to_json();
        for field in ["clock", "idle_min", "rare_at", "legend_at"] {
            assert!(!text.contains(field), "{field}: {text}");
        }
    }

    /// The most a count holds in memory is the most it reads back as:
    /// a ledger stopped at the top doesn't come back at zero.
    #[test]
    fn a_count_at_the_top_round_trips() {
        let ledger = Ledger {
            clock: MINUTES_MAX,
            idle_min: MINUTES_MAX,
            rare_at: MINUTES_MAX,
            legend_at: MINUTES_MAX,
            ..furnished()
        };
        // Its summary for `--dump` reads it too, with or without a date.
        let at_top = ledger.summary(None);
        assert_eq!(at_top.clock_min, MINUTES_MAX);
        assert_eq!(at_top.pity.rare.since_new_min, 0);
        let _ = ledger.summary(NaiveDate::from_ymd_opt(2026, 8, 1));
        assert_eq!(Ledger::from_json(&ledger.to_json()), Ok(ledger));
    }

    /// A value this build can't take reads as zero, and the rest of the
    /// record still reads.
    #[test]
    fn a_garbled_clock_reads_as_zero() {
        for field in ["clock", "idle_min", "rare_at", "legend_at"] {
            for garbage in [
                r#""noon""#,
                "-5",
                "2.5",
                "1e30",
                "18446744073709551615",
                "1099511627776",
                "null",
                "[1]",
                "{}",
            ] {
                let text = furnished().to_json().replace(
                    r#""bought_on":6}"#,
                    &format!(r#""bought_on":6,"{field}":{garbage}}}"#),
                );
                assert_eq!(
                    Ledger::from_json(&text),
                    Ok(furnished()),
                    "{field}: {garbage}"
                );
            }
        }
        // The top still reads.
        let text = furnished().to_json().replace(
            r#""bought_on":6}"#,
            r#""bought_on":6,"clock":1099511627775}"#,
        );
        assert_eq!(Ledger::from_json(&text).unwrap().clock, MINUTES_MAX);
    }

    /// The date of her calendar comes last, after her clock and
    /// counters, as "YYYY-MM-DD", and reads back as written; none is
    /// written until there is one.
    #[test]
    fn the_calendar_date_round_trips() {
        let ledger = Ledger {
            calendar_on: NaiveDate::from_ymd_opt(2027, 2, 3),
            ..timed()
        };
        let text = ledger.to_json();
        assert!(
            text.ends_with(r#""legend_at":17,"calendar_on":"2027-02-03"}"#),
            "{text}"
        );
        assert_eq!(Ledger::from_json(&text), Ok(ledger.clone()));
        let alone = Ledger {
            calendar_on: NaiveDate::from_ymd_opt(2040, 12, 31),
            ..furnished()
        };
        let text = alone.to_json();
        assert!(
            text.ends_with(r#""bought_on":6,"calendar_on":"2040-12-31"}"#),
            "{text}"
        );
        assert_eq!(Ledger::from_json(&text), Ok(alone));
        assert!(!timed().to_json().contains("calendar_on"));
        assert!(!furnished().to_json().contains("calendar_on"));
        // An older build keeps every piece of it.
        assert_eq!(
            as_an_older_build_reads(&ledger.to_json()),
            as_an_older_build_reads(&furnished().to_json())
        );
    }

    /// A date this build can't read reads as none (the day's entry is
    /// owed again), and the rest of the record still reads.
    #[test]
    fn a_garbled_calendar_date_reads_as_none() {
        for garbage in [
            r#""2027-02-30""#,
            r#""2027-2-3x""#,
            r#""tomorrow""#,
            r#""""#,
            "20270203",
            "null",
            "[2027, 2, 3]",
            r#"{"y": 2027}"#,
            "true",
        ] {
            let text = furnished().to_json().replace(
                r#""bought_on":6}"#,
                &format!(r#""bought_on":6,"calendar_on":{garbage}}}"#),
            );
            assert_eq!(Ledger::from_json(&text), Ok(furnished()), "{garbage}");
        }
    }

    /// The rare things she has shown come last, after her calendar's
    /// date, by their stable ids in the order she showed them, and read
    /// back as written; none is written until there's one; an older
    /// build keeps every piece of it.
    #[test]
    fn what_shes_seen_round_trips() {
        let seen = |keys: &[&str]| keys.iter().map(|k| (*k).to_owned()).collect::<Vec<_>>();
        let ledger = Ledger {
            calendar_on: NaiveDate::from_ymd_opt(2027, 2, 3),
            seen: seen(&["escalator", "dream"]),
            ..timed()
        };
        let text = ledger.to_json();
        assert!(
            text.ends_with(r#""calendar_on":"2027-02-03","seen":["escalator","dream"]}"#),
            "{text}"
        );
        assert_eq!(Ledger::from_json(&text), Ok(ledger.clone()));
        assert_eq!(
            ledger.seen_scripts(),
            [ScriptId::Escalator, ScriptId::Dream]
        );
        let alone = Ledger {
            seen: seen(&["no-melon"]),
            ..furnished()
        };
        let text = alone.to_json();
        assert!(
            text.ends_with(r#""bought_on":6,"seen":["no-melon"]}"#),
            "{text}"
        );
        assert_eq!(Ledger::from_json(&text), Ok(alone));
        assert!(!timed().to_json().contains("seen"));
        assert!(!Ledger::new(3).to_json().contains("seen"));
        assert_eq!(
            as_an_older_build_reads(&ledger.to_json()),
            as_an_older_build_reads(&furnished().to_json())
        );
    }

    /// Marking something rare seen: the first time, it's in her record
    /// and her pity for its tier starts again from her idle minutes then;
    /// shown again later, nothing changes (her pity runs on from the
    /// first). A Legendary's pity is its own; an ungated script has none
    /// and isn't recorded.
    #[test]
    fn marking_seen_once_starts_her_pity_once() {
        use super::super::rarity::Rarity;
        let mut ledger = Ledger {
            idle_min: 100,
            rare_at: 7,
            legend_at: 8,
            ..timed()
        };
        assert!(ledger.mark_seen("no-melon", Rarity::Rare));
        assert_eq!((ledger.rare_at, ledger.legend_at), (100, 8));
        ledger.idle_min = 250;
        assert!(!ledger.mark_seen("no-melon", Rarity::Rare));
        assert_eq!(ledger.seen, ["no-melon"]);
        assert_eq!(ledger.rare_at, 100, "the first showing's");
        assert!(ledger.mark_seen("test-legend", Rarity::Legendary));
        assert_eq!((ledger.rare_at, ledger.legend_at), (100, 250));
        ledger.idle_min = 300;
        assert!(!ledger.mark_seen("test-legend", Rarity::Legendary));
        assert_eq!(ledger.legend_at, 250);
        for tier in [Rarity::Common, Rarity::Uncommon] {
            assert!(!ledger.mark_seen("snack", tier), "{tier:?}");
        }
        assert_eq!(ledger.seen, ["no-melon", "test-legend"]);
        assert_eq!((ledger.rare_at, ledger.legend_at), (100, 250));
    }

    /// Pity counters read ahead of her idle minutes (a garbled record, or
    /// a clock set back) round-trip as written and read as no pity yet.
    #[test]
    fn pity_ahead_of_her_idle_minutes_round_trips() {
        let ledger = Ledger {
            idle_min: 5,
            rare_at: 10,
            legend_at: 3000,
            ..timed()
        };
        let read = Ledger::from_json(&ledger.to_json()).unwrap();
        assert_eq!(read, ledger);
        assert_eq!(
            super::super::rarity::Pity::of(read.idle_min, read.rare_at, read.legend_at),
            super::super::rarity::Pity::default()
        );
    }

    /// What she's seen is read entry by entry: an id this build doesn't
    /// know (a later build's), anything that isn't an id, and an id
    /// twice are skipped, the rest kept in order; a value that isn't a
    /// list at all reads as nothing seen. The rest of the record reads
    /// either way.
    #[test]
    fn garbage_in_what_shes_seen_is_skipped() {
        let with = |value: &str| {
            furnished().to_json().replace(
                r#""bought_on":6}"#,
                &format!(r#""bought_on":6,"seen":{value}}}"#),
            )
        };
        for (garbage, kept) in [
            (r#"["dream","unicorn","scary"]"#, &["dream", "scary"][..]),
            (
                r#"[1, null, "dream", {"id": "scary"}, ["scary"]]"#,
                &["dream"],
            ),
            (r#"["scary","scary","dream","scary"]"#, &["scary", "dream"]),
            (r#"["Dream", "DREAM", " dream", ""]"#, &[]),
            (r#""dream""#, &[]),
            ("17", &[]),
            ("null", &[]),
            ("true", &[]),
            (r#"{"dream": true}"#, &[]),
            ("[]", &[]),
        ] {
            let ledger = Ledger::from_json(&with(garbage)).unwrap();
            assert_eq!(ledger.seen, kept, "{garbage}");
            assert_eq!(
                Ledger {
                    seen: Vec::new(),
                    ..ledger
                },
                furnished(),
                "{garbage}"
            );
        }
    }

    /// An older build, which knows none of them, still keeps every
    /// piece of a record with them all set.
    #[test]
    fn an_older_build_reads_past_the_clock() {
        assert_eq!(
            as_an_older_build_reads(&timed().to_json()),
            as_an_older_build_reads(&furnished().to_json())
        );
        assert_eq!(as_an_older_build_reads(&timed().to_json()).len(), 3);
    }

    /// That her wall clock was sent comes last, only once it has been,
    /// and reads back; anything but `true` there reads as not yet,
    /// without failing the record.
    #[test]
    fn the_clock_sent_round_trips() {
        let text = furnished().to_json();
        assert!(!text.contains("clock_sent"), "{text}");
        let sent = Ledger {
            clock_sent: true,
            ..timed()
        };
        let text = sent.to_json();
        assert!(text.ends_with(r#","clock_sent":true}"#), "{text}");
        assert_eq!(Ledger::from_json(&text), Ok(sent));
        for garbage in ["false", "1", "\"yes\"", "null", "[true]", "{}"] {
            let text = furnished().to_json().replace(
                r#""bought_on":6}"#,
                &format!(r#""bought_on":6,"clock_sent":{garbage}}}"#),
            );
            assert_eq!(Ledger::from_json(&text), Ok(furnished()), "{garbage}");
        }
    }

    /// Her door's wall comes after what she has seen and before her clock
    /// sent, only once she has one, and reads back.
    #[test]
    fn a_door_wall_round_trips() {
        let text = furnished().to_json();
        assert!(!text.contains("door"), "{text}");
        let mut doored = Ledger {
            clock_sent: true,
            seen: vec!["dream".to_owned()],
            ..furnished()
        };
        doored.home.door = Some(DoorWall {
            strip: Strip::Bottom(Nook::Users),
            side: Side::Right,
        });
        let text = doored.to_json();
        assert!(
            text.ends_with(
                r#""seen":["dream"],"door":{"strip":{"Bottom":"Users"},"side":"Right"},"clock_sent":true}"#
            ),
            "{text}"
        );
        assert_eq!(Ledger::from_json(&text), Ok(doored.clone()));
        assert_eq!(doored.summary(None).door.as_deref(), Some("Users right"));
        assert_eq!(furnished().summary(None).door, None);
    }

    /// A door's wall this build can't read reads as none, without failing
    /// the record.
    #[test]
    fn a_garbled_door_wall_reads_as_none() {
        for garbage in [
            // (Read as none by any lenient field; here for completeness.)
            "null",
            "1",
            "\"Users\"",
            "[]",
            "{}",
            r#"{"strip":{"Bottom":"Chat"},"side":"Right"}"#,
            r#"{"strip":{"Bottom":"Users"},"side":"Up"}"#,
        ] {
            let text = furnished().to_json().replace(
                r#""bought_on":6}"#,
                &format!(r#""bought_on":6,"door":{garbage}}}"#),
            );
            assert!(text.contains(r#""door""#), "{garbage}: not in {text}");
            assert_eq!(Ledger::from_json(&text), Ok(furnished()), "{garbage}");
        }
    }

    /// An older build reads a record with her door's wall as one without.
    #[test]
    fn an_older_build_reads_past_the_door() {
        let mut doored = furnished();
        doored.home.door = Some(DoorWall {
            strip: Strip::Bottom(Nook::Playlist),
            side: Side::Left,
        });
        assert!(doored.to_json().contains("door"));
        assert_eq!(
            as_an_older_build_reads(&doored.to_json()),
            as_an_older_build_reads(&furnished().to_json())
        );
    }

    /// Her wall clock and window are pieces older builds don't know: they
    /// skip them, the two claim no room's pane there, and every piece
    /// they do know keeps its room's; this build reads them back where
    /// they hang. Both are kept in the living room by kind.
    #[test]
    fn an_older_build_skips_the_clock_and_the_window() {
        let mut ledger = Ledger::new(3);
        for (item, nook, at) in [
            (Furniture::Clock, Nook::Users, 0),
            (Furniture::Window, Nook::List, 1000),
            (Furniture::Sofa, Nook::Playlist, 0),
            (Furniture::Tv, Nook::Playlist, 1000),
        ] {
            assert!(ledger.home.add(Prop {
                anchor: Some(Anchor {
                    side: Side::Left,
                    offset: at / 100,
                }),
                ..Prop::new(item, nook, at, Facing::Right)
            }));
        }
        ledger.clock_sent = true;
        let text = ledger.to_json();
        assert!(!text.contains(r#"["Living","Users"]"#), "{text}");
        assert_eq!(
            as_an_older_build_reads(&text),
            [
                (Furniture::Sofa, Nook::Playlist, 0),
                (Furniture::Tv, Nook::Playlist, 1000),
            ]
        );
        assert_eq!(Ledger::from_json(&text), Ok(ledger));
        for item in [Furniture::Clock, Furniture::Window] {
            assert!(!item.legacy(), "{item:?}");
            assert_eq!(RoomKind::of(item), RoomKind::Living, "{item:?}");
        }
        assert_eq!(
            Furniture::ALL
                .iter()
                .filter(|item| item.legacy())
                .collect::<Vec<_>>(),
            Furniture::ALL[..8].iter().collect::<Vec<_>>()
        );
        // This build's names for them are the ones they know.
        for item in Furniture::ALL {
            let name = serde_json::to_value(item).unwrap();
            assert_eq!(
                item.legacy(),
                OLDEST_KINDS.contains(&name.as_str().unwrap()),
                "{item:?}"
            );
        }
    }

    #[test]
    fn another_version_is_not_read() {
        let text = furnished()
            .to_json()
            .replace("\"version\":1", "\"version\":2");
        assert!(Ledger::from_json(&text).is_err());
        assert!(Ledger::from_json("{}").is_err(), "no version");
        assert!(Ledger::from_json("not json").is_err());
    }

    /// A record, and each piece and stand in it, is an object: serde
    /// would read a struct from an array by position, so `[1,2]` would
    /// otherwise be a record of version 1 with master seed 2, and a
    /// positional piece a piece. Nothing writes either.
    #[test]
    fn only_objects_are_read_as_records() {
        for garbage in ["[]", "[1]", "[1,2]", "[1,2,3,4]", "1", r#""text""#, "null"] {
            assert!(Ledger::from_json(garbage).is_err(), "{garbage}");
        }
        let text = r#"{"version":1,"rooms":[["Living","Users"]],
            "props":[["Sofa",300,"Right",false],{"item":"Tv","at":100}],
            "anchors":[["Tv",{"Bottom":"Users"},{"side":"Left","offset":3}]]}"#;
        let ledger = Ledger::from_json(text).unwrap();
        let items: Vec<Furniture> = ledger.home.props.iter().map(|p| p.item).collect();
        assert_eq!(items, [Furniture::Tv]);
        assert_eq!(ledger.home.props[0].anchor, None);
    }

    #[test]
    fn unknown_pieces_and_rooms_are_skipped() {
        let text = r#"{
            "version": 1, "master_seed": 5, "visits": 2,
            "rooms": [["Living", "Users"], ["Attic", "List"]],
            "props": [
                {"item": "Sofa", "at": 300, "facing": "Right"},
                {"item": "Piano", "at": 100},
                {"item": "Bed", "at": 100}
            ]
        }"#;
        let ledger = Ledger::from_json(text).unwrap();
        // The bed's room has no pane on record: it's dropped.
        assert_eq!(ledger.home.props.len(), 1);
        assert_eq!(ledger.home.props[0].strip, Strip::Bottom(Nook::Users));
        assert!(ledger.home.owns(Furniture::Sofa));
        assert_eq!((ledger.master_seed, ledger.visits), (5, 2));
    }

    #[test]
    fn missing_fields_default() {
        let ledger = Ledger::from_json(r#"{"version": 1}"#).unwrap();
        assert_eq!(ledger, Ledger::new(0));
    }

    #[test]
    fn the_first_visit_is_seeded_by_the_master_seed() {
        let ledger = Ledger::new(99);
        assert_eq!(ledger.visit_seed(0), 99);
        assert_ne!(ledger.visit_seed(1), ledger.visit_seed(2));
    }

    #[test]
    fn it_persists_in_local_storage() {
        let storage = crate::storage::Storage::open_in_memory().unwrap();
        assert_eq!(Ledger::load(&storage).unwrap(), None, "never visited");
        let ledger = furnished();
        ledger.save(&storage).unwrap();
        assert_eq!(Ledger::load(&storage).unwrap(), Some(ledger));
    }

    #[test]
    fn it_does_not_clobber_her_arrival_setting() {
        let storage = crate::storage::Storage::open_in_memory().unwrap();
        let settings = crate::config::Settings {
            houseguest: crate::config::Houseguest::Off,
            ..storage.load_settings().unwrap()
        };
        storage.save_settings(&settings).unwrap();
        let ledger = furnished();
        ledger.save(&storage).unwrap();
        assert_eq!(
            storage.load_settings().unwrap().houseguest,
            crate::config::Houseguest::Off
        );
        storage.save_settings(&settings).unwrap();
        assert_eq!(Ledger::load(&storage).unwrap(), Some(ledger));
    }

    #[test]
    fn a_ledger_under_the_old_key_resets_the_setting() {
        let storage = crate::storage::Storage::open_in_memory().unwrap();
        storage
            .set_setting("houseguest", Some(&furnished().to_json()))
            .unwrap();
        assert_eq!(
            storage.load_settings().unwrap().houseguest,
            crate::config::Houseguest::default()
        );
    }
}
