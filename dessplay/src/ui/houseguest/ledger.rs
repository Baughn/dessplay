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

use serde::{Deserialize, Serialize};

use super::room::{Anchor, Furniture, Home, Nook, Prop, Strip};
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

    /// The seed of visit number `visit` (the first is the master seed
    /// itself).
    pub(super) fn visit_seed(&self, visit: u64) -> u64 {
        self.master_seed ^ visit.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    /// Read from JSON; `Err` for text that isn't a record this build can
    /// read (malformed, or another version).
    pub fn from_json(text: &str) -> Result<Self, String> {
        let raw: Raw = serde_json::from_str(text).map_err(|e| e.to_string())?;
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
            .filter_map(|v| serde_json::from_value::<SavedAnchor>(v).ok())
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
            .filter_map(|v| serde_json::from_value::<SavedProp>(v).ok())
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
    // Decor claims no room's pane: older builds don't know it.
    for prop in home.props.iter().filter(|p| !p.item.decor()) {
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
            | Furniture::Poster => Self::Living,
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::room::Side;
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
                // It knows no decor: an unknown piece is skipped.
                let item = serde_json::from_value::<Furniture>(prop["item"].clone())
                    .ok()
                    .filter(|item| !item.decor())?;
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

    #[test]
    fn another_version_is_not_read() {
        let text = furnished()
            .to_json()
            .replace("\"version\":1", "\"version\":2");
        assert!(Ledger::from_json(&text).is_err());
        assert!(Ledger::from_json("{}").is_err(), "no version");
        assert!(Ledger::from_json("not json").is_err());
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
