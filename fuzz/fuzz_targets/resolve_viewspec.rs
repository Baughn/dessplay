#![no_main]
use arbitrary::Arbitrary;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dessplay::tui::resolve::resolve_input;
use dessplay_core::view_spec::ViewSpec;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    spec: ViewSpec,
    key_byte: u8,
    char_byte: u8,
    modifier_bits: u8,
}

fn make_key_event(input: &Input) -> KeyEvent {
    let code = match input.key_byte % 14 {
        0 => KeyCode::Char(char::from(input.char_byte)),
        1 => KeyCode::Enter,
        2 => KeyCode::Esc,
        3 => KeyCode::Tab,
        4 => KeyCode::Backspace,
        5 => KeyCode::Delete,
        6 => KeyCode::Up,
        7 => KeyCode::Down,
        8 => KeyCode::Left,
        9 => KeyCode::Right,
        10 => KeyCode::Home,
        11 => KeyCode::End,
        12 => KeyCode::BackTab,
        _ => KeyCode::Char(char::from(input.char_byte.wrapping_add(b'a'))),
    };
    KeyEvent::new(code, KeyModifiers::from_bits_truncate(input.modifier_bits))
}

fuzz_target!(|input: Input| {
    let key = make_key_event(&input);
    let _ = resolve_input(key, &input.spec);
});
