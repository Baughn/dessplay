#![no_main]

use arbitrary::Arbitrary;
use dessplay_core::crdt::playlist::Playlist;
use dessplay_core::protocol::PlaylistAction;
use dessplay_core::types::FileId;
use libfuzzer_sys::fuzz_target;

/// Constrained playlist input: 5 file IDs, 16 timestamps. The small space
/// forces meaningful interactions (adds, removes, moves on the same files)
/// that the generic convergence target almost never achieves.
#[derive(Arbitrary, Debug)]
struct Input {
    /// Each op: (timestamp_byte, action_encoding_byte)
    ops: Vec<(u8, u8)>,
    perm_seed: u64,
}

fn make_fid(n: u8) -> FileId {
    let mut id = [0u8; 16];
    id[0] = n % 5;
    FileId(id)
}

fn decode_action(encoded: u8) -> PlaylistAction {
    let file = encoded & 0x07;
    let anchor = (encoded >> 3) & 0x07;
    let action_type = (encoded >> 6) & 0x03;
    let fid = make_fid(file);
    let after = if anchor % 6 == 0 {
        None
    } else {
        Some(make_fid(anchor))
    };

    match action_type {
        0 => PlaylistAction::Add {
            file_id: fid,
            after,
        },
        1 => PlaylistAction::Remove { file_id: fid },
        _ => PlaylistAction::Move {
            file_id: fid,
            after,
        },
    }
}

fn shuffle_with_seed<T>(items: &mut [T], seed: u64) {
    let len = items.len();
    let mut state = seed;
    for i in (1..len).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state as usize) % (i + 1);
        items.swap(i, j);
    }
}

fuzz_target!(|input: Input| {
    if input.ops.is_empty() {
        return;
    }

    let ops: Vec<(u64, PlaylistAction)> = input
        .ops
        .iter()
        .map(|(ts, enc)| (u64::from(*ts % 16), decode_action(*enc)))
        .collect();

    // Apply in original order
    let mut pl_a = Playlist::new();
    for (ts, action) in &ops {
        pl_a.apply(*ts, action.clone());
    }

    // Apply in shuffled order
    let mut shuffled = ops.clone();
    shuffle_with_seed(&mut shuffled, input.perm_seed);

    let mut pl_b = Playlist::new();
    for (ts, action) in &shuffled {
        pl_b.apply(*ts, action.clone());
    }

    assert_eq!(pl_a.snapshot(), pl_b.snapshot());
});
