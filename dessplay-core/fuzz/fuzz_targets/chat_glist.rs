//! Chat GList ordering: appends from multiple replicas converge to one
//! sequence regardless of delivery order, with no losses or duplicates.

#![no_main]

use dessplay_core::CrdtState;
use dessplay_core::types::{ChatMessage, SharedTimestamp, UserId};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (Vec<(u8, u8)>, u8)| {
    let (messages, rotation) = input;
    if messages.is_empty() || messages.len() > 24 {
        return;
    }

    // Three replicas append concurrently without seeing each other.
    let mut replicas = vec![CrdtState::new(), CrdtState::new(), CrdtState::new()];
    let mut ops = Vec::new();
    for (i, (sender, text)) in messages.iter().enumerate() {
        let index = *sender as usize % replicas.len();
        let op = replicas[index].append_chat(ChatMessage {
            timestamp: SharedTimestamp(i as u64),
            sender: UserId::new(format!("user{index}")),
            text: format!("m{text}"),
        });
        ops.push(op);
    }

    // Two observers receive the ops in different rotations (GList inserts
    // are order-free), plus duplicates.
    let rotation = rotation as usize % ops.len();
    let mut observer_a = CrdtState::new();
    for op in &ops {
        observer_a.apply(op.clone());
    }
    let mut observer_b = CrdtState::new();
    for op in ops.iter().cycle().skip(rotation).take(ops.len()) {
        observer_b.apply(op.clone());
        observer_b.apply(op.clone());
    }

    let chat_a = observer_a.view().chat;
    assert_eq!(chat_a.len(), messages.len(), "lost or duplicated messages");
    assert_eq!(chat_a, observer_b.view().chat);
});
