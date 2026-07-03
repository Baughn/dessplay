//! An arbitrary sequence of raw mpv IPC lines through the translation
//! layer must never panic. This mirrors `read_loop`: each line is either
//! unparseable JSON (skipped, same as the real reader) or a `Value` fed
//! to `translate` against one running [`Translate`] accumulator — so the
//! cross-message state (pause/path dedup, the seek-reply request-id
//! match, EOF edge-triggering) gets exercised across many messages, not
//! just validated one message at a time.

#![no_main]

use std::sync::atomic::AtomicBool;

use dessplay::player::mpv::{Translate, translate};
use libfuzzer_sys::fuzz_target;

/// One simulated read_loop iteration: a candidate IPC line, plus whatever
/// our own `loading` flag happens to be set to at that moment (it is
/// toggled by `load()`/`file-loaded` independently of the message stream
/// in the real actor, so the fuzz input drives it directly instead).
#[derive(Debug, arbitrary::Arbitrary)]
struct Step {
    loading: bool,
    line: String,
}

fuzz_target!(|steps: Vec<Step>| {
    let mut state = Translate::default();
    for step in steps {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&step.line) else {
            continue;
        };
        let loading = AtomicBool::new(step.loading);
        let _ = translate(&msg, &mut state, &loading);
    }
});
