//! Offline retained-heap census. Reads only the supplied sync database.
//! Run with: cargo run --release -p dessplay --example memory_census -- /path/to/dessplay.sync.db
//! Counts requested Rust allocation bytes, not allocator overhead, native allocations or RSS.
use dessplay_core::CrdtState;
use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static GLOBAL: Counting = Counting;
#[cfg(not(target_os = "windows"))]
const BASE: mimalloc::MiMalloc = mimalloc::MiMalloc;
#[cfg(target_os = "windows")]
const BASE: std::alloc::System = std::alloc::System;

// SAFETY: every operation forwards the original pointer and layout to the
// same allocator. Accounting uses only atomics and cannot allocate recursively.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { BASE.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Relaxed);
            ALLOCS.fetch_add(1, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        ALLOCS.fetch_sub(1, Relaxed);
        unsafe { BASE.dealloc(p, layout) };
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let next = unsafe { BASE.realloc(p, layout, new_size) };
        if !next.is_null() {
            LIVE.fetch_add(new_size, Relaxed);
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        next
    }
}
fn measure<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let before = LIVE.load(Relaxed);
    let count = ALLOCS.load(Relaxed);
    let result = std::hint::black_box(f());
    let bytes = LIVE.load(Relaxed) as i64 - before as i64;
    let allocations = ALLOCS.load(Relaxed) as i64 - count as i64;
    println!("{label:38} {bytes:12} bytes {allocations:9} allocations");
    result
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Requested Rust heap bytes (native SQLite allocations excluded)");
    let path = std::env::args()
        .nth(1)
        .ok_or("expected sync database path")?;
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let blob: Vec<u8> = conn.query_row(
        "SELECT state FROM crdt_state WHERE room='default'",
        [],
        |r| r.get(0),
    )?;
    println!("Serialized snapshot: {} bytes", blob.len());
    let state = measure("decode retained CRDT", || CrdtState::decode_snapshot(&blob))?;
    drop(blob);
    let _ = measure("CRDT playlist", || state.playlist.clone());
    let _ = measure("CRDT watched", || state.watched.clone());
    let _ = measure("CRDT now_playing", || state.now_playing.clone());
    let _ = measure("CRDT seek_authority", || state.seek_authority.clone());
    let _ = measure("CRDT playback_intent", || state.playback_intent.clone());
    let _ = measure("CRDT series_preference", || state.series_preference.clone());
    let _ = measure("CRDT manual_override", || state.manual_override.clone());
    let _ = measure("CRDT file_availability", || state.file_availability.clone());
    let _ = measure("CRDT anidb_metadata", || state.anidb_metadata.clone());
    let _ = measure("CRDT series_relations", || state.series_relations.clone());
    let _ = measure("CRDT file_catalog", || state.file_catalog.clone());
    let _ = measure("CRDT list_entries", || state.list_entries.clone());
    let _ = measure("CRDT list_next_ep", || state.list_next_ep.clone());
    let _ = measure("CRDT lookup_requests", || state.lookup_requests.clone());
    let _ = measure("CRDT chat", || state.chat.clone());
    let _ = measure("CRDT playback_position", || state.playback_position.clone());
    let _ = measure("CRDT acknowledged_absent", || {
        state.acknowledged_absent.clone()
    });
    let _ = measure("CRDT marquee", || state.marquee.clone());
    // Upstream's context-bearing iterator clones clocks; use it only here,
    // outside the measured operation. Exclude the temporary Vec's storage.
    for (name, clocks) in [
        (
            "metadata",
            state
                .anidb_metadata
                .iter()
                .map(|ctx| ctx.rm_clock)
                .collect::<Vec<_>>(),
        ),
        (
            "catalogue",
            state
                .file_catalog
                .iter()
                .map(|ctx| ctx.rm_clock)
                .collect::<Vec<_>>(),
        ),
    ] {
        let dots: usize = clocks.iter().map(|c| c.dots.len()).sum();
        let before = LIVE.load(Relaxed);
        let copy = std::hint::black_box(clocks.clone());
        let bytes = LIVE.load(Relaxed) - before - std::mem::size_of_val(copy.as_slice());
        println!(
            "{name} entry clocks: {} clocks, {dots} dots, {bytes} heap bytes",
            clocks.len()
        );
        drop(copy);
    }
    let view = measure("resolved StateView", || state.view());
    let _ = measure("view playlist", || view.playlist.clone());
    let _ = measure("view watched", || view.watched.clone());
    let _ = measure("view now_playing", || view.now_playing);
    let _ = measure("view seek_authority", || view.seek_authority.clone());
    let _ = measure("view playback_intent", || view.playback_intent);
    let _ = measure("view series_preference", || view.series_preference.clone());
    let _ = measure("view manual_override", || view.manual_override.clone());
    let _ = measure("view file_availability", || view.file_availability.clone());
    let _ = measure("view anidb_metadata", || view.anidb_metadata.clone());
    let _ = measure("view series_relations", || view.series_relations.clone());
    let _ = measure("view file_catalog", || view.file_catalog.clone());
    let _ = measure("view list_entries", || view.list_entries.clone());
    let _ = measure("view list_next_ep", || view.list_next_ep.clone());
    let _ = measure("view lookup_requests", || view.lookup_requests.clone());
    let _ = measure("view chat", || view.chat.clone());
    let _ = measure("view playback_position", || view.playback_position.clone());
    let _ = measure("view acknowledged_absent", || {
        view.acknowledged_absent.clone()
    });
    let _ = measure("view marquee", || view.marquee.clone());
    println!("count playlist: {}", view.playlist.len());
    println!("count watched: {}", view.watched.len());
    println!("count file_availability: {}", view.file_availability.len());
    println!("count anidb_metadata: {}", view.anidb_metadata.len());
    println!("count series_relations: {}", view.series_relations.len());
    println!("count file_catalog: {}", view.file_catalog.len());
    println!("count list_entries: {}", view.list_entries.len());
    println!("count lookup_requests: {}", view.lookup_requests.len());
    println!("count chat: {}", view.chat.len());
    let _ = measure("franchises", || dessplay_core::franchise::franchises(&view));
    let mut ui = measure("empty UI", || {
        dessplay::ui::app::Ui::new(
            dessplay_core::types::UserId::new("profile"),
            dessplay::config::Settings::default(),
            vec![],
        )
    });
    let view = std::sync::Arc::new(view);
    measure("apply UI snapshot", || {
        ui.apply_snapshot(dessplay::ui::app::UiSnapshot {
            view: view.clone(),
            ..Default::default()
        })
    });
    drop(view);
    let mut terminal = measure("terminal buffers 160x48", || {
        tuirealm::ratatui::Terminal::new(tuirealm::ratatui::backend::TestBackend::new(160, 48))
    })?;
    measure("first UI draw", || {
        terminal.draw(|frame| ui.draw(frame)).map(|_| ())
    })?;
    for _ in 0..3 {
        measure("build and replace UI snapshot", || {
            let next = std::sync::Arc::new(state.view());
            ui.apply_snapshot(dessplay::ui::app::UiSnapshot {
                view: next,
                ..Default::default()
            });
        });
        measure("repeat UI draw", || {
            terminal.draw(|frame| ui.draw(frame)).map(|_| ())
        })?;
    }
    Ok(())
}
