#![no_main]
use arbitrary::Arbitrary;
use dessplay::tui::renderer::render;
use dessplay_core::view_spec::ViewSpec;
use libfuzzer_sys::fuzz_target;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[derive(Arbitrary, Debug)]
struct Input {
    spec: ViewSpec,
    width: u8,
    height: u8,
}

fuzz_target!(|input: Input| {
    let width = (input.width % 200).max(1) as u16;
    let height = (input.height % 100).max(1) as u16;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("TestBackend is infallible");
    let _ = terminal.draw(|frame| render(&input.spec, frame));
});
