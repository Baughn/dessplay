#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use proptest::prelude::*;
use tuirealm::ratatui::{Terminal, backend::TestBackend, layout::Rect};

#[test]
fn defaults_are_valid_and_transferable() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<LayoutBundle>();
    assert!(LayoutBundle::builtin().is_ok());
}
#[test]
fn export_never_overwrites_and_missing_files_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    LayoutBundle::init(dir.path()).unwrap();
    std::fs::write(dir.path().join("style.css"), "#form { padding: 1ch; }").unwrap();
    LayoutBundle::init(dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("style.css")).unwrap(),
        "#form { padding: 1ch; }"
    );
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        LayoutBundle::load(empty.path()).unwrap().revision,
        LayoutBundle::builtin().unwrap().revision
    );
}
#[test]
fn rejects_incomplete_unknown_and_mistyped_sources() {
    for source in [
        "<templates version=\"2\"></templates>",
        "<templates version=\"1\"><template name=\"form\"><column>",
        "<templates version=\"1\"><template name=\"form\"><script/></template></templates>",
        "<templates version=\"1\"><template name=\"form\"><text bind=\"editing\"/></template></templates>",
        "<templates version=\"1\"><template name=\"form\"><column><slot name=\"body\"/><slot name=\"body\"/></column></template></templates>",
        "<templates version=\"1\"><template name=\"form\"><column><box id=\"a\"/><box id=\"a\"/></column></template></templates>",
    ] {
        let dir = tempfile::tempdir().unwrap();
        LayoutBundle::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("templates/form.xml"), source).unwrap();
        assert!(LayoutBundle::load(dir.path()).is_err(), "{source}");
    }
    for css in [
        "#form { width: 4px; }",
        "#form { position: absolute; }",
        "#form { width: 1ch;",
        "#form:hover { color: red; }",
        "#form { color: var(--missing); }",
        "#form { --a: var(--b); --b: var(--a); color: var(--a); }",
        "#form { color: url(https://example.com); }",
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("style.css"), css).unwrap();
        let error = LayoutBundle::load(dir.path()).unwrap_err();
        assert!(error.line > 0 && error.column > 0, "{error}");
    }
}
#[test]
fn cascade_specificity_inheritance_and_custom_variables() {
    let dir = tempfile::tempdir().unwrap();
    LayoutBundle::init(dir.path()).unwrap();
    std::fs::write(dir.path().join("style.css"), "#form { --accent: #123456; color: var(--accent); } column > scroll > slot { color: blue; } #form slot { color: red; } slot { color: green; }").unwrap();
    let bundle = LayoutBundle::load(dir.path()).unwrap();
    let scene = Renderer::new(bundle)
        .arrange(
            "form",
            Rect::new(0, 0, 30, 15),
            &Presentation::default().slot("body", 0, 0),
        )
        .unwrap();
    assert_eq!(
        scene.style("body").fg,
        Some(tuirealm::ratatui::style::Color::Red)
    );
}
proptest! {
    #[test]
    fn geometry_is_translation_invariant_and_contained(width in 0u16..150, height in 0u16..80, x in 0u16..200, y in 0u16..100) {
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let data = Presentation::default().text("title","Form").slot("header",0,3).slot("body",0,0).slot("notes",0,1).slot("save",0,1);
        let a = renderer.arrange("form",Rect::new(0,0,width,height),&data).unwrap();
        let area = Rect::new(x,y,width,height);
        let b = renderer.arrange("form",area,&data).unwrap();
        for name in ["header","body","notes","save"] {
            let one = a.slot(name); let two = b.slot(name);
            prop_assert_eq!(one.width,two.width); prop_assert_eq!(one.height,two.height);
            if !two.is_empty() { prop_assert_eq!(two.x,one.x+x); prop_assert_eq!(two.y,one.y+y); prop_assert_eq!(two.intersection(area),two); }
        }
    }
    #[test]
    fn text_fragment_ranges_are_valid(text in ".{0,100}", width in 0usize..80) {
        for fragment in measure_text(&text,width,0,2,true,false) {
            prop_assert!(fragment.source.end <= text.chars().count());
            prop_assert!(fragment.width + fragment.indent <= width);
        }
    }
}
#[test]
fn rendering_uses_measured_bounds_at_nonzero_origin() {
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let scene = renderer
        .arrange(
            "form",
            Rect::new(7, 3, 30, 12),
            &Presentation::default()
                .text("title", "Settings")
                .slot("body", 0, 0)
                .slot("save", 0, 1),
        )
        .unwrap();
    terminal.draw(|f| scene.paint(f)).unwrap();
    assert_eq!(terminal.backend().buffer()[(7, 3)].symbol(), "┌");
    assert_eq!(scene.slot("save"), Rect::new(8, 13, 28, 1));
}

#[test]
fn helpers_validate_in_the_callers_schema_and_cycles_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("templates")).unwrap();
    let file = dir.path().join("templates/custom.xml");
    std::fs::write(&file, r#"<templates version="1"><template name="form"><use template="section"/></template><template name="section"><column><slot name="body"/></column></template></templates>"#).unwrap();
    assert!(LayoutBundle::load(dir.path()).is_ok());
    std::fs::write(&file, r#"<templates version="1"><template name="form"><use template="section"/></template><template name="section"><use template="form"/></template></templates>"#).unwrap();
    assert!(
        LayoutBundle::load(dir.path())
            .unwrap_err()
            .message
            .contains("cycle")
    );
}

#[test]
fn rejects_incomplete_comments_functions_and_invalid_ancestor_state_variables() {
    let dir = tempfile::tempdir().unwrap();
    for css in [
        "/* an incomplete editor save",
        "#form { --unused: var(--x; }",
        "#form { --size: 1ch; } #form:focus { --size: red; } #form slot { width: var(--size); }",
    ] {
        std::fs::write(dir.path().join("style.css"), css).unwrap();
        assert!(LayoutBundle::load(dir.path()).is_err(), "{css}");
    }
}

#[test]
fn huge_authored_box_paints_only_its_original_clipped_edges() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("templates")).unwrap();
    std::fs::write(dir.path().join("templates/form.xml"), r#"<templates version="1"><template name="form"><column><box style="width: 65535ch; height: 65535lh; border: 1"/></column></template></templates>"#).unwrap();
    let mut renderer = Renderer::new(LayoutBundle::load(dir.path()).unwrap());
    let scene = renderer
        .arrange("form", Rect::new(0, 0, 10, 5), &Presentation::default())
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(10, 5)).unwrap();
    terminal.draw(|f| scene.paint(f)).unwrap();
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "┌");
    assert_eq!(terminal.backend().buffer()[(9, 4)].symbol(), " ");
}

#[test]
fn measured_rows_are_reused_until_the_content_or_width_changes() {
    let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
    let first = renderer.measured_text("a long line with Unicode 日本語", 12);
    let same = renderer.measured_text("a long line with Unicode 日本語", 12);
    let resized = renderer.measured_text("a long line with Unicode 日本語", 13);
    assert!(std::sync::Arc::ptr_eq(&first, &same));
    assert!(!std::sync::Arc::ptr_eq(&first, &resized));
}

#[test]
fn drag_sizes_are_local_revision_scoped_and_legacy_import_happens_once() {
    let storage = crate::storage::Storage::open_in_memory().unwrap();
    let builtin = LayoutBundle::builtin().unwrap();
    let mut sizes = LayoutSettings::default();
    let legacy = crate::config::PaneLayout {
        chat_width: 65,
        ..Default::default()
    };
    assert!(sizes.activate(&builtin, legacy));
    assert_eq!(sizes.shares("builtin")["chat-column"], 6500);
    sizes.save(&storage).unwrap();
    let mut restored = LayoutSettings::load(&storage).unwrap();
    assert_eq!(restored, sizes);
    assert!(!restored.activate(&builtin, Default::default()));
    let mut changed = builtin.clone();
    changed.revision.push('x');
    assert!(restored.activate(&changed, legacy));
    assert!(restored.shares("builtin").is_empty());
    restored.save(&storage).unwrap();
    let mut restored = LayoutSettings::load(&storage).unwrap();
    assert!(!restored.activate(&changed, legacy));
    assert!(
        restored.shares("builtin").is_empty(),
        "legacy sizes never reappear"
    );
    let dir = tempfile::tempdir().unwrap();
    let custom = LayoutBundle::load(dir.path()).unwrap();
    restored.activate(&custom, legacy);
    let source = LayoutSettings::source(&custom);
    restored.set(
        &source,
        "panes",
        [("chat-column".into(), 7000), ("right-column".into(), 3000)].into(),
    );
    restored.save(&storage).unwrap();
    storage
        .save_settings(&crate::config::Settings::default())
        .unwrap();
    assert_eq!(
        LayoutSettings::load(&storage).unwrap(),
        restored,
        "ordinary settings saves cannot overwrite drag sizes"
    );
    std::fs::write(
        dir.path().join("style.css"),
        "#chat-column { flex-basis: 40%; }",
    )
    .unwrap();
    assert!(restored.activate(&LayoutBundle::load(dir.path()).unwrap(), legacy));
    assert!(
        restored.shares(&source).is_empty(),
        "file changes win across restart"
    );
}

#[test]
fn resizable_containers_require_stable_children_and_flex_layout() {
    for content in [
        r#"<row resizable="true"><slot name="chat"/><slot name="users"/></row>"#,
        r#"<row id="s" resizable="true"><slot name="chat"/><slot id="u" name="users"/></row>"#,
        r#"<row id="s" resizable="false"><slot id="c" name="chat"/><slot id="u" name="users"/></row>"#,
        r#"<row id="s" resizable="true" style="display: grid"><slot id="c" name="chat"/><slot id="u" name="users"/></row>"#,
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("templates")).unwrap();
        std::fs::write(
            dir.path().join("templates/app.xml"),
            format!(
                "<templates version=\"1\"><template name=\"app\">{content}</template></templates>"
            ),
        )
        .unwrap();
        assert!(LayoutBundle::load(dir.path()).is_err(), "{content}");
    }
}

proptest! {
    #[test]
    fn split_handles_translate_with_paint_and_trade_only_adjacent_children(width in 30u16..150, height in 20u16..80, x in 0u16..100, y in 0u16..100, pointer in 0u16..200) {
        use tuirealm::ratatui::layout::Position;
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let data = Presentation::default().boolean("separate-subtitles", true);
        let a = renderer.arrange("app", Rect::new(0,0,width,height), &data).unwrap();
        let b = renderer.arrange("app", Rect::new(x,y,width,height), &data).unwrap();
        prop_assert_eq!(a.splits.len(), b.splits.len());
        for (a,b) in a.splits.iter().zip(b.splits.iter()) {
            prop_assert_eq!(&a.id,&b.id);
            prop_assert_eq!(b.handle.x,a.handle.x+x);
            prop_assert_eq!(b.handle.y,a.handle.y+y);
            prop_assert_eq!(b.handle.width,a.handle.width);
            prop_assert_eq!(b.handle.height,a.handle.height);
            let one = a.drag(Position::new(pointer,pointer));
            let two = b.drag(Position::new(pointer+x,pointer+y));
            prop_assert_eq!(&one,&two);
            prop_assert_eq!(one.values().map(|n| u32::from(*n)).sum::<u32>(),10_000);
            let initial = a.drag(Position::new(a.handle.x+1,a.handle.y+1));
            prop_assert!(one.iter().filter(|(id,n)| initial[*id] != **n).count() <= 2);
        }
    }
}

proptest! {
    #[test]
    fn wrapped_collection_paint_and_hits_agree_at_arbitrary_origins(width in 2u16..65, height in 1u16..25, x in 0u16..8, y in 0u16..8, center in 0usize..20) {
        use tuirealm::ratatui::style::{Color, Style};
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("templates")).unwrap();
        std::fs::write(dir.path().join("templates/form-row.xml"), r#"<templates version="1"><template name="form-row"><column style="padding: 1ch"><text bind="value" style="white-space: normal"/></column></template></templates>"#).unwrap();
        let mut renderer = Renderer::new(LayoutBundle::load(dir.path()).unwrap());
        let rows = (0..20).map(|i| PresentedRow {
            key: i.to_string(),
            data: Presentation::default().text("value", "界 words and more words ".repeat(i % 3 + 1)).style("value", Style::default().bg(Color::Indexed(i as u8 + 1))),
            gap_after: false,
        }).collect::<Vec<_>>();
        let area = Rect::new(x,y,width,height);
        let mut terminal = Terminal::new(TestBackend::new(x+width+2,y+height+2)).unwrap();
        let mut interactions = RenderedCollection::default();
        let frame = terminal.draw(|frame| {
            interactions = renderer.paint_collection(frame,area,"form-row",&rows,Some(center),Some(center),Style::default()).unwrap();
        }).unwrap();
        for yy in 0..frame.area.height {
            for xx in 0..frame.area.width {
                if let Color::Indexed(index) = frame.buffer[(xx,yy)].bg {
                    prop_assert_eq!(interactions.hit(xx,yy),Some(usize::from(index-1)));
                    prop_assert!(area.contains(tuirealm::ratatui::layout::Position::new(xx,yy)));
                }
                if !area.contains(tuirealm::ratatui::layout::Position::new(xx,yy)) {
                    prop_assert!(interactions.hit(xx,yy).is_none());
                }
            }
        }
    }
}
