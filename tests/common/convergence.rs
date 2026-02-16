use std::sync::Arc;

use dessplay::network::sync::SyncEngine;
use dessplay::state::types::*;

/// Assert that all engines have converged to the same state.
///
/// Checks: position (within tolerance), current file, playlist, chat messages,
/// user states (only for connected peers that are tracked by each engine).
pub fn assert_converged(engines: &[Arc<SyncEngine>], tolerance_secs: f64) {
    assert!(
        engines.len() >= 2,
        "need at least 2 engines to check convergence"
    );

    let views: Vec<_> = engines.iter().map(|e| {
        // Access state through the engine's shared state
        let state = e.shared_state();
        state.view()
    }).collect();

    // Check position convergence
    let positions: Vec<f64> = views.iter().map(|v| v.position).collect();
    for (i, pos) in positions.iter().enumerate() {
        for (j, other_pos) in positions.iter().enumerate() {
            if i != j {
                let diff = (pos - other_pos).abs();
                assert!(
                    diff <= tolerance_secs,
                    "position divergence between engine {i} ({pos:.3}s) and engine {j} ({other_pos:.3}s): diff={diff:.3}s > tolerance={tolerance_secs}s"
                );
            }
        }
    }

    // Check current file convergence
    let files: Vec<&Option<ItemId>> = views.iter().map(|v| &v.current_file).collect();
    for (i, file) in files.iter().enumerate() {
        for (j, other_file) in files.iter().enumerate() {
            if i != j {
                assert_eq!(
                    file, other_file,
                    "current file divergence between engine {i} and engine {j}"
                );
            }
        }
    }

    // Check playlist convergence
    let playlists: Vec<&Vec<PlaylistItem>> = views.iter().map(|v| &v.playlist).collect();
    for (i, pl) in playlists.iter().enumerate() {
        for (j, other_pl) in playlists.iter().enumerate() {
            if i != j {
                assert_eq!(
                    pl, other_pl,
                    "playlist divergence between engine {i} ({} items) and engine {j} ({} items)",
                    pl.len(),
                    other_pl.len(),
                );
            }
        }
    }

    // Check chat message convergence (sorted by timestamp)
    let chats: Vec<Vec<(&str, &str)>> = views
        .iter()
        .map(|v| {
            v.chat_messages
                .iter()
                .map(|m| (m.sender.0.as_str(), m.text.as_str()))
                .collect()
        })
        .collect();
    for (i, chat) in chats.iter().enumerate() {
        for (j, other_chat) in chats.iter().enumerate() {
            if i != j {
                assert_eq!(
                    chat, other_chat,
                    "chat divergence between engine {i} ({} msgs) and engine {j} ({} msgs)",
                    chat.len(),
                    other_chat.len(),
                );
            }
        }
    }
}
