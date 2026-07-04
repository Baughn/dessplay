//! JSON rendering of client state for `dessplay --dump`.
//!
//! `StateView` derives `Serialize`, but `serde_json` cannot render it
//! directly: several maps are keyed by tuples or by `Ed2kHash`
//! (`[u8; 16]`) — neither is a JSON object key — and a bare `Ed2kHash`
//! value serializes as a 16-element byte array rather than the hex string
//! everyone actually reads. This module rebuilds the view into a
//! query-friendly JSON document — hashes as hex everywhere, tuple-keyed
//! maps as arrays of objects — without touching `StateView`'s own
//! `Serialize` impl, which `CrdtState::view_hash` hashes canonically and
//! must not change.
//!
//! The result is a single JSON object on stdout (logs go to stderr) so it
//! can be sliced with `jq`. `--section` trims it to just the parts a query
//! needs, keeping the common case off the multi-megabyte metadata/catalog
//! maps.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dessplay_core::StateSnapshot;
use dessplay_core::playlist::PlaylistEntry;
use dessplay_core::state::StateView;
use dessplay_core::types::{Ed2kHash, FileHashInfo, ListEntryId};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::config::Settings;

/// Selectable top-level sections, in dump order. `settings`/`media_roots`
/// are document-level; the rest are fields of the `state` object.
pub const SECTIONS: &[&str] = &[
    "settings",
    "media_roots",
    "playlist",
    "watched",
    "now_playing",
    "seek_authority",
    "playback_intent",
    "series_preference",
    "manual_override",
    "file_availability",
    "anidb_metadata",
    "series_relations",
    "file_catalog",
    "list_entries",
    "list_next_ep",
    "lookup_requests",
    "chat",
    "playback_position",
    "acknowledged_absent",
];

/// The sections that live under the `state` object (everything except the
/// two document-level ones).
const STATE_SECTIONS: &[&str] = &[
    "playlist",
    "watched",
    "now_playing",
    "seek_authority",
    "playback_intent",
    "series_preference",
    "manual_override",
    "file_availability",
    "anidb_metadata",
    "series_relations",
    "file_catalog",
    "list_entries",
    "list_next_ep",
    "lookup_requests",
    "chat",
    "playback_position",
    "acknowledged_absent",
];

/// Which sections to emit. `all` (the default, no `--section` given)
/// includes everything; otherwise only the named set.
#[derive(Debug)]
pub struct Selection {
    requested: Option<Vec<String>>,
}

impl Selection {
    /// Emit every section.
    pub fn all() -> Self {
        Self { requested: None }
    }

    /// Parse a `--section` list. Empty means all; unknown names are an
    /// error naming the valid set (so a typo fails loudly, not silently
    /// empty).
    pub fn parse(list: &[String]) -> Result<Self, String> {
        if list.is_empty() {
            return Ok(Self::all());
        }
        for name in list {
            if !SECTIONS.contains(&name.as_str()) {
                return Err(format!(
                    "unknown --section {name:?}; valid sections: {}",
                    SECTIONS.join(", ")
                ));
            }
        }
        Ok(Self {
            requested: Some(list.to_vec()),
        })
    }

    fn wants(&self, name: &str) -> bool {
        match &self.requested {
            None => true,
            Some(list) => list.iter().any(|s| s == name),
        }
    }

    /// Whether any requested section lives under `state` — gates the
    /// (non-trivial) state resolution so a settings-only dump skips it.
    fn wants_state(&self) -> bool {
        STATE_SECTIONS.iter().any(|s| self.wants(s))
    }
}

/// Build the full dump document for the given inputs.
pub fn build(
    database: &str,
    settings: &Settings,
    media_roots: &[PathBuf],
    snapshot: Option<&StateSnapshot>,
    sections: &Selection,
) -> Result<Value, serde_json::Error> {
    let mut doc = Map::new();
    doc.insert("database".into(), json!(database));
    if sections.wants("settings") {
        doc.insert("settings".into(), settings_json(settings));
    }
    if sections.wants("media_roots") {
        let roots: Vec<String> = media_roots
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        doc.insert("media_roots".into(), json!(roots));
    }
    match snapshot {
        None => {
            doc.insert("epoch".into(), Value::Null);
            doc.insert("state".into(), Value::Null);
        }
        Some(snap) => {
            doc.insert("epoch".into(), json!(snap.epoch.0));
            if sections.wants_state() {
                doc.insert("state".into(), state_json(&snap.state.view(), sections)?);
            }
        }
    }
    Ok(Value::Object(doc))
}

/// Settings as JSON. The password is **redacted** (only its presence is
/// shown) — unlike the old `Debug` dump, which printed it in cleartext.
fn settings_json(s: &Settings) -> Value {
    json!({
        "username": s.username,
        "server": s.server,
        "password": if s.password.is_some() { "<set>" } else { "<unset>" },
        "player": format!("{:?}", s.player),
        "ready_on_startup": s.ready_on_startup,
        "cache_retention": format!("{:?}", s.cache_retention),
        "upload_limit": s.upload_limit,
        "subtitle_mode": format!("{:?}", s.subtitle_mode),
        "series_sort": format!("{:?}", s.series_sort),
        "auto_download": s.auto_download,
        "irc_enabled": s.irc_enabled,
        "irc_server": s.irc_server,
        "irc_tls": s.irc_tls,
        "irc_channel": s.irc_channel,
    })
}

fn state_json(view: &StateView, sel: &Selection) -> Result<Value, serde_json::Error> {
    let mut m = Map::new();
    if sel.wants("playlist") {
        let entries: Vec<Value> = view.playlist.iter().map(playlist_entry_json).collect();
        m.insert("playlist".into(), Value::Array(entries));
    }
    if sel.wants("watched") {
        m.insert("watched".into(), hash_keyed(&view.watched)?);
    }
    if sel.wants("now_playing") {
        m.insert("now_playing".into(), opt_hash(view.now_playing.as_ref()));
    }
    if sel.wants("seek_authority") {
        m.insert(
            "seek_authority".into(),
            serde_json::to_value(&view.seek_authority)?,
        );
    }
    if sel.wants("playback_intent") {
        m.insert(
            "playback_intent".into(),
            serde_json::to_value(view.playback_intent)?,
        );
    }
    if sel.wants("series_preference") {
        let mut rows = Vec::new();
        for ((user, entry), pref) in &view.series_preference {
            rows.push(json!({
                "user": user.0,
                // Stringified: serde_json can't safely round-trip a raw
                // u128 (see `u128_keyed`).
                "entry": entry.0.to_string(),
                "state": serde_json::to_value(pref.state)?,
                "set_by": pref.set_by.as_ref().map(|u| &u.0),
            }));
        }
        m.insert("series_preference".into(), Value::Array(rows));
    }
    if sel.wants("manual_override") {
        // Keyed by `UserId` (a string newtype) — a valid JSON object key.
        m.insert(
            "manual_override".into(),
            serde_json::to_value(&view.manual_override)?,
        );
    }
    if sel.wants("file_availability") {
        let mut rows = Vec::new();
        for ((user, hash), avail) in &view.file_availability {
            rows.push(json!({
                "user": user.0,
                "hash": hash.to_string(),
                "availability": serde_json::to_value(avail)?,
            }));
        }
        m.insert("file_availability".into(), Value::Array(rows));
    }
    if sel.wants("anidb_metadata") {
        m.insert("anidb_metadata".into(), hash_keyed(&view.anidb_metadata)?);
    }
    if sel.wants("series_relations") {
        // Keyed by `AniDbSeriesId` (a `u32` newtype) — serde_json renders
        // it as a string key.
        m.insert(
            "series_relations".into(),
            serde_json::to_value(&view.series_relations)?,
        );
    }
    if sel.wants("file_catalog") {
        m.insert("file_catalog".into(), hash_keyed(&view.file_catalog)?);
    }
    if sel.wants("list_entries") {
        m.insert("list_entries".into(), u128_keyed(&view.list_entries)?);
    }
    if sel.wants("list_next_ep") {
        m.insert("list_next_ep".into(), u128_keyed(&view.list_next_ep)?);
    }
    if sel.wants("lookup_requests") {
        let rows: Vec<Value> = view
            .lookup_requests
            .iter()
            .map(file_hash_info_json)
            .collect();
        m.insert("lookup_requests".into(), Value::Array(rows));
    }
    if sel.wants("chat") {
        m.insert("chat".into(), serde_json::to_value(&view.chat)?);
    }
    if sel.wants("playback_position") {
        m.insert(
            "playback_position".into(),
            serde_json::to_value(&view.playback_position)?,
        );
    }
    if sel.wants("acknowledged_absent") {
        let rows: Vec<Value> = view
            .acknowledged_absent
            .iter()
            .map(|(hash, user)| json!({ "hash": hash.to_string(), "user": user.0 }))
            .collect();
        m.insert("acknowledged_absent".into(), Value::Array(rows));
    }
    Ok(Value::Object(m))
}

fn playlist_entry_json(entry: &PlaylistEntry) -> Value {
    json!({
        "hash": entry.hash.to_string(),
        "filename": entry.state.filename,
        "added_by": entry.state.added_by.0,
        "size_bytes": entry.state.size_bytes,
        "duration_millis": entry.state.duration_millis,
    })
}

fn file_hash_info_json(info: &FileHashInfo) -> Value {
    json!({
        "hash": info.hash.to_string(),
        "size": info.size,
        "filename": info.filename,
        "mtime": info.mtime,
        "series_hint": info.series_hint,
    })
}

fn opt_hash(hash: Option<&Ed2kHash>) -> Value {
    match hash {
        Some(h) => json!(h.to_string()),
        None => Value::Null,
    }
}

/// An `Ed2kHash`-keyed map as a JSON object with hex-string keys.
fn hash_keyed<V: Serialize>(map: &BTreeMap<Ed2kHash, V>) -> Result<Value, serde_json::Error> {
    let mut out = Map::new();
    for (hash, value) in map {
        out.insert(hash.to_string(), serde_json::to_value(value)?);
    }
    Ok(Value::Object(out))
}

/// A `ListEntryId`-keyed map as a JSON object with the id stringified —
/// `serde_json` does not accept the underlying 128-bit integer as a key.
fn u128_keyed<V: Serialize>(map: &BTreeMap<ListEntryId, V>) -> Result<Value, serde_json::Error> {
    let mut out = Map::new();
    for (key, value) in map {
        out.insert(key.0.to_string(), serde_json::to_value(value)?);
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use dessplay_core::types::{SeriesPreference, SeriesWatchState, UserId};

    fn hash(byte: u8) -> Ed2kHash {
        Ed2kHash([byte; 16])
    }

    /// A view exercising the cases `serde_json` cannot render directly:
    /// a tuple-keyed map, a hash-keyed map, and a bare-hash register.
    fn sample_view() -> StateView {
        let mut watched = BTreeMap::new();
        watched.insert(hash(0xcd), true);
        let mut series_preference = BTreeMap::new();
        series_preference.insert(
            (UserId::new("Baughn"), ListEntryId(18302)),
            SeriesPreference {
                state: SeriesWatchState::Watching,
                set_by: None,
            },
        );
        StateView {
            now_playing: Some(hash(0xab)),
            watched,
            series_preference,
            ..Default::default()
        }
    }

    #[test]
    fn now_playing_renders_as_hex_not_a_byte_array() {
        let json = state_json(&sample_view(), &Selection::all()).unwrap();
        assert_eq!(
            json["now_playing"],
            json!("abababababababababababababababab")
        );
    }

    #[test]
    fn watched_uses_hex_string_keys() {
        let json = state_json(&sample_view(), &Selection::all()).unwrap();
        let watched = json["watched"].as_object().unwrap();
        assert_eq!(watched["cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"], json!(true));
    }

    #[test]
    fn series_preference_is_an_array_of_user_series_state_rows() {
        let json = state_json(&sample_view(), &Selection::all()).unwrap();
        let rows = json["series_preference"].as_array().unwrap();
        assert_eq!(
            rows,
            &vec![
                json!({ "user": "Baughn", "entry": "18302", "state": "Watching", "set_by": null })
            ]
        );
    }

    #[test]
    fn selection_emits_only_requested_state_sections() {
        let sel = Selection::parse(&["now_playing".to_string()]).unwrap();
        let json = state_json(&sample_view(), &sel).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["now_playing"]);
    }

    #[test]
    fn unknown_section_is_rejected_with_the_valid_list() {
        let err = Selection::parse(&["bogus".to_string()]).unwrap_err();
        assert!(err.contains("bogus"));
        assert!(err.contains("series_preference"));
    }

    #[test]
    fn empty_selection_means_all() {
        let json = state_json(&sample_view(), &Selection::parse(&[]).unwrap()).unwrap();
        // All state sections present, even the empty ones.
        assert!(json.as_object().unwrap().contains_key("file_catalog"));
        assert!(json.as_object().unwrap().contains_key("chat"));
    }
}
