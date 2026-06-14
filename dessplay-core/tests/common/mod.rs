//! Proptest strategies over the shared `test_support` script types.

// Each test crate compiles this module but uses only some strategies.
#![allow(dead_code)]

use dessplay_core::test_support::{ClusterEvent, ScriptOp, ScriptStep};
use proptest::prelude::*;

/// Strategy for one scripted intention. Domains are tiny on purpose so
/// histories collide on the same keys.
pub fn arb_script_op() -> impl Strategy<Value = ScriptOp> {
    prop_oneof![
        (any::<u8>(), proptest::option::of(any::<u8>()))
            .prop_map(|(file, after)| ScriptOp::AddPlaylist { file, after }),
        (any::<u8>(), proptest::option::of(any::<u8>()))
            .prop_map(|(file, after)| ScriptOp::MovePlaylist { file, after }),
        any::<u8>().prop_map(|file| ScriptOp::RemovePlaylist { file }),
        (any::<u8>(), any::<bool>())
            .prop_map(|(file, watched)| ScriptOp::SetWatched { file, watched }),
        proptest::option::of(any::<u8>()).prop_map(|file| ScriptOp::SetNowPlaying { file }),
        any::<u8>().prop_map(|authority| ScriptOp::SetSeekAuthority { authority }),
        any::<bool>().prop_map(|playing| ScriptOp::SetIntent { playing }),
        (any::<u8>(), any::<u8>(), any::<bool>()).prop_map(|(user, series, watching)| {
            ScriptOp::SetSeriesPreference {
                user,
                series,
                watching,
            }
        }),
        (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(user, kind, set_by)| {
            ScriptOp::SetManualOverride { user, kind, set_by }
        }),
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u16>()).prop_map(
            |(user, file, kind, progress_bps)| ScriptOp::SetFileAvailability {
                user,
                file,
                kind,
                progress_bps,
            }
        ),
        (any::<u8>(), any::<bool>(), any::<u8>()).prop_map(|(file, known, series)| {
            ScriptOp::SetMetadata {
                file,
                known,
                series,
            }
        }),
        (any::<u8>(), any::<u8>())
            .prop_map(|(series, target)| ScriptOp::SetRelations { series, target }),
        any::<u8>().prop_map(|file| ScriptOp::SetFileCatalog { file }),
        (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(entry, status, note)| {
            ScriptOp::PutListEntry {
                entry,
                status,
                note,
            }
        }),
        (any::<u8>(), any::<u8>(), any::<bool>()).prop_map(|(entry, ep, available)| {
            ScriptOp::SetNextEp {
                entry,
                ep,
                available,
            }
        }),
        any::<u8>().prop_map(|file| ScriptOp::RequestLookup { file }),
        any::<u8>().prop_map(|text| ScriptOp::Chat { text }),
        (any::<u8>(), any::<u32>())
            .prop_map(|(user, millis)| ScriptOp::SetPosition { user, millis }),
    ]
}

/// Strategy for a scripted step. Timestamps draw from a small window to
/// force same-timestamp LWW ties.
pub fn arb_step() -> impl Strategy<Value = ScriptStep> {
    (any::<u8>(), 0u16..32, arb_script_op()).prop_map(|(actor, ts, op)| ScriptStep {
        actor,
        ts,
        op,
    })
}

/// Strategy for one cluster scheduling/mutation event. Weighted toward
/// client ops with enough polls/deliveries mixed in to create deep
/// divergence before the final flush.
pub fn arb_cluster_event() -> impl Strategy<Value = ClusterEvent> {
    prop_oneof![
        4 => (any::<u8>(), 0u16..32, arb_script_op())
            .prop_map(|(client, ts, op)| ClusterEvent::ClientOp { client, ts, op }),
        1 => (0u16..32, arb_script_op()).prop_map(|(ts, op)| ClusterEvent::ServerOp { ts, op }),
        3 => any::<u8>().prop_map(|lane| ClusterEvent::ServerPoll { lane }),
        3 => (any::<u8>(), 0u8..6)
            .prop_map(|(client, count)| ClusterEvent::Deliver { client, count }),
        1 => any::<u8>().prop_map(|client| ClusterEvent::Reconnect { client }),
    ]
}
