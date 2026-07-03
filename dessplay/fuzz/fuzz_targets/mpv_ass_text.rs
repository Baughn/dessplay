//! Arbitrary subtitle text through the ASS override-tag stripper must
//! never panic (or hang — the brace/drawing-mode state machine has a
//! "run to end of string" fallback for an unclosed `{`, which is exactly
//! the kind of thing a corrupt/adversarial subtitle track could trigger)
//! and must never *grow* the text: every code path either copies a
//! character through unchanged, collapses a multi-char escape into one
//! space, or drops an override block entirely.

#![no_main]

use dessplay::player::mpv::parse_ass_full;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|raw: &str| {
    let (text, _speaker) = parse_ass_full(raw);
    assert!(
        text.len() <= raw.len(),
        "stripped text grew: {raw:?} -> {text:?}"
    );
});
