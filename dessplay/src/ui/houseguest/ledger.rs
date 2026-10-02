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

use serde::{Deserialize, Serialize};

use super::room::{Anchor, Furniture, Home, Nook, Prop, RoomKind, Strip};
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
            let room = rooms.iter().find(|&&(k, _)| k == prop.item.room());
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

/// Which pane each room is in, as older builds read it: the pane of its
/// first piece's strip, or, where another room has that pane, the first
/// pane none has (each room needs a pane of its own there).
fn rooms(home: &Home) -> Vec<(RoomKind, Nook)> {
    let mut out: Vec<(RoomKind, Nook)> = Vec::new();
    for prop in &home.props {
        let kind = prop.item.room();
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
                let item: Furniture = serde_json::from_value(prop["item"].clone()).unwrap();
                let at = prop["at"].as_u64().unwrap() as u16;
                let &(_, nook) = rooms.iter().find(|&&(k, _)| k == item.room())?;
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

    #[test]
    fn a_ledger_round_trips() {
        let ledger = furnished();
        assert_eq!(Ledger::from_json(&ledger.to_json()), Ok(ledger));
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
